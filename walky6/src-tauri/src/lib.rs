use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Listener, Manager};

const TOOLBAR_H: u32 = 90;

enum StoreMsg {
    Frontpage { order: String, resp: std::sync::mpsc::Sender<anyhow::Result<String>> },
    GetEntry { host: String, path: String, resp: std::sync::mpsc::Sender<anyhow::Result<Option<(String, Vec<u8>, String)>>> },
}

fn start_store_thread(store_dir: PathBuf) -> std::sync::mpsc::Sender<StoreMsg> {
    let (tx, rx) = std::sync::mpsc::channel::<StoreMsg>();
    std::thread::spawn(move || {
        let mut store = smol::block_on(
            sneakerweb::PersistentStore::new(&store_dir),
        )
        .expect("failed to open sneakerweb storage");
        while let Ok(msg) = rx.recv() {
            match msg {
                StoreMsg::Frontpage { order, resp } => {
                    let result = smol::block_on(
                        sneakerweb::serve::render_frontpage_with_store(&mut store, &order),
                    );
                    let _ = resp.send(result);
                }
                StoreMsg::GetEntry { host, path, resp } => {
                    let result = smol::block_on(
                        sneakerweb::serve::get_entry_with_store(&mut store, &host, &path),
                    );
                    let _ = resp.send(result);
                }
            }
        }
    });
    tx
}

pub struct ContentWebview(pub std::sync::Arc<Mutex<Option<tauri::Webview>>>);

pub struct ImportState {
    pub is_importing: AtomicBool,
    pub phase: AtomicU32,
    pub processed_bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    pub processed_entries: AtomicU64,
}

impl ImportState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            is_importing: AtomicBool::new(false),
            phase: AtomicU32::new(0),
            processed_bytes: AtomicU64::new(0),
            total_bytes: AtomicU64::new(0),
            processed_entries: AtomicU64::new(0),
        })
    }
}

fn ensure_sneakerweb_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SNEAKERWEB_DIR") {
        let path = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&path);
        return path;
    }
    if let Ok(cwd) = std::env::current_dir() {
        let mut local_dir = cwd;
        local_dir.push(".sneakerweb_store");
        let _ = std::fs::create_dir_all(&local_dir);
        return local_dir;
    }
    PathBuf::from(".sneakerweb_store")
}

fn rewrite_urls(body: &[u8], content_type: &str) -> Vec<u8> {
    let is_rewritable = content_type.contains("text/html")
        || content_type.contains("text/css")
        || content_type.contains("application/javascript")
        || content_type.contains("text/javascript")
        || content_type.contains("application/json");
    if !is_rewritable {
        return body.to_vec();
    }
    let body_str = String::from_utf8_lossy(body);
    static RE: std::sync::OnceLock<regex_lite::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        let re_str = r#"https?://(sneakerweb|[a-fA-F0-9]{64})\.localhost(?::\d+)?(/[^"'\s>]*)?"#;
        regex_lite::Regex::new(re_str).expect("failed to compile regex")
    });
    let rewritten = re.replace_all(&body_str, |caps: &regex_lite::Captures| {
        let subdomain = caps.get(1).map_or("", |m| m.as_str());
        let path = caps.get(2).map_or("", |m| m.as_str());
        if subdomain == "sneakerweb" {
            #[cfg(target_os = "windows")]
            { format!("http://sneaker.localhost/home{path}") }
            #[cfg(not(target_os = "windows"))]
            { format!("sneaker://home{path}") }
        } else {
            #[cfg(target_os = "windows")]
            { format!("http://sneaker.localhost/{subdomain}{path}") }
            #[cfg(not(target_os = "windows"))]
            { format!("sneaker://{subdomain}{path}") }
        }
    });
    rewritten.into_owned().into_bytes()
}

#[derive(Clone, serde::Serialize)]
pub struct ImportProgress {
    pub phase: String,
    pub processed: u64,
    pub total: u64,
    pub processed_entries: u64,
    pub message: String,
}

fn progress_to_event(handle: &sneakerweb::import::ProgressHandle) -> ImportProgress {
    let phase = match handle.phase.load(Ordering::Relaxed) {
        sneakerweb::import::PHASE_IDLE => "idle",
        sneakerweb::import::PHASE_DECODING => "decoding",
        sneakerweb::import::PHASE_IMPORTING => "importing",
        sneakerweb::import::PHASE_DONE => "done",
        _ => "unknown",
    };
    let processed = handle.processed_bytes.load(Ordering::Relaxed);
    let total = handle.total_bytes.load(Ordering::Relaxed);
    let processed_entries = handle.processed_entries.load(Ordering::Relaxed);
    let message = match phase {
        "decoding" => "Decoding entries...",
        "importing" => "Importing entries...",
        "done" => "Import complete",
        _ => "",
    };
    ImportProgress {
        phase: phase.to_string(),
        processed,
        total,
        processed_entries,
        message: message.to_string(),
    }
}

fn run_import_internal(app: AppHandle, file_path: PathBuf, import_state: Arc<ImportState>) -> anyhow::Result<()> {
    import_state.is_importing.store(true, Ordering::Relaxed);
    import_state.phase.store(1, Ordering::Relaxed);
    import_state.processed_bytes.store(0, Ordering::Relaxed);
    import_state.processed_entries.store(0, Ordering::Relaxed);

    let args = sneakerweb::import::ImportArgs {
        src: file_path,
        mode: None,
    };
    let handle = sneakerweb::import::ProgressHandle::new();
    import_state.total_bytes.store(0, Ordering::Relaxed);

    let poller_handle = handle.clone();
    let app_clone = app.clone();
    let state_clone = import_state.clone();

    let poller = std::thread::spawn(move || loop {
        let progress = progress_to_event(&poller_handle);
        if progress.total > 0 {
            state_clone.total_bytes.store(progress.total, Ordering::Relaxed);
        }
        state_clone.phase.store(poller_handle.phase.load(Ordering::Relaxed), Ordering::Relaxed);
        state_clone.processed_bytes.store(progress.processed, Ordering::Relaxed);
        state_clone.processed_entries.store(progress.processed_entries, Ordering::Relaxed);

        let is_done = progress.phase == "done";
        let _ = app_clone.emit("import-progress", progress);
        if is_done {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    });

    let result = smol::block_on(sneakerweb::import::import_sneak(&args, &handle));
    let _ = poller.join();

    if result.is_ok() {
        import_state.phase.store(3, Ordering::Relaxed);
    } else {
        import_state.phase.store(4, Ordering::Relaxed);
    }

    result
}

#[tauri::command]
fn get_base_url() -> String {
    "sneaker://home/".to_string()
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Debug)]
pub struct NavPayload {
    pub url: String,
    pub action: String,
}

#[tauri::command]
async fn webview_navigated(
    app: AppHandle,
    url: String,
    action: String,
) -> Result<(), String> {
    println!("[Rust webview_navigated] Received event from webview. URL: {}, Action: {}", url, action);
    match app.emit("webview-navigated", NavPayload { url: url.clone(), action: action.clone() }) {
        Ok(_) => {
            println!("[Rust webview_navigated] Successfully emitted webview-navigated event to all webviews");
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("Failed to emit webview-navigated: {e}");
            println!("[Rust webview_navigated] {err_msg}");
            Err(err_msg)
        }
    }
}


#[tauri::command]
async fn navigate_content(
    content_wv: tauri::State<'_, ContentWebview>,
    path: String,
) -> Result<(), String> {
    let parsed = if path.starts_with("sneaker://") {
        url::Url::parse(&path).map_err(|e| format!("invalid URL: {e}"))?
    } else {
        let base = "sneaker://home";
        let clean_path = if path.starts_with('/') { path } else { format!("/{path}") };
        url::Url::parse(&format!("{base}{clean_path}")).map_err(|e| format!("invalid URL: {e}"))?
    };

    #[cfg(target_os = "windows")]
    let parsed = if parsed.scheme() == "sneaker" {
        let host = parsed.host_str().unwrap_or("home");
        let path_and_query = match parsed.query() {
            Some(q) => format!("{}?{}", parsed.path(), q),
            None => parsed.path().to_string(),
        };
        url::Url::parse(&format!("http://sneaker.localhost/{}{}", host, path_and_query))
            .map_err(|e| e.to_string())?
    } else {
        parsed
    };

    let wv = content_wv.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = *wv {
        wv.navigate(parsed).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn go_back_content(content_wv: tauri::State<'_, ContentWebview>) -> Result<(), String> {
    let wv = content_wv.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = *wv {
        wv.eval("window.history.back()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn go_forward_content(content_wv: tauri::State<'_, ContentWebview>) -> Result<(), String> {
    let wv = content_wv.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = *wv {
        wv.eval("window.history.forward()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn pick_file(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let file_path = app
        .dialog()
        .file()
        .add_filter("Sneaker files", &["snk"])
        .set_title("Import .snk file")
        .blocking_pick_file();

    match file_path {
        Some(path) => Ok(path.to_string()),
        None => Err("File selection cancelled".to_string()),
    }
}

#[tauri::command]
async fn import_file(app: AppHandle, file_path: String, import_state: tauri::State<'_, Arc<ImportState>>) -> Result<(), String> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {file_path}"));
    }
    run_import_internal(app, path, import_state.inner().clone()).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    {
        // Keep hardware acceleration (DMABUF renderer) enabled for better performance (60/120 FPS).
        // Only disable it if experiencing rendering artifacts or crashes by setting the environment variable externally.
        // if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
        //     std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        // }
    }

    let import_state = ImportState::new();

    let dir = ensure_sneakerweb_dir();
    unsafe {
        std::env::set_var("SNEAKERWEB_DIR", &dir);
    }

    let store_tx = start_store_thread(dir);

    let protocol_import_state = import_state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(import_state)
        .manage(ContentWebview(std::sync::Arc::new(Mutex::new(None))))
        .register_asynchronous_uri_scheme_protocol("sneaker", move |ctx, req, responder| {
            let import_state = protocol_import_state.clone();
            let app_handle = ctx.app_handle().clone();

            let tx = store_tx.clone();
            std::thread::spawn(move || {
                let original_path = req.uri().path().to_string();
                let host_buf = req.uri().host().unwrap_or("home").to_string();
                let path_str = original_path.clone();

                let is_asset = path_str.starts_with("/assets/")
                    || path_str.starts_with("/src/")
                    || path_str.starts_with("/node_modules/")
                    || path_str.starts_with("/@vite/")
                    || path_str.starts_with("/@id/")
                    || path_str.starts_with("/@fs/")
                    || path_str == "/@react-refresh";

                #[cfg(target_os = "windows")]
                let (host_buf, path_str) = {
                    if host_buf == "localhost" {
                        if !is_asset && path_str != "/__progress__" && path_str != "/__progress_api__" {
                            let key = path_str.trim_start_matches('/');
                            if let Some((first_segment, rest)) = key.split_once('/') {
                                (first_segment.to_string(), format!("/{}", rest))
                            } else {
                                (key.to_string(), "/".to_string())
                            }
                        } else {
                            (host_buf, path_str)
                        }
                    } else {
                        (host_buf, path_str)
                    }
                };

                let host = &host_buf;
                let path = &path_str;

                // 1. Serving __progress__ page
                if path == "/__progress__" {
                    let response = match app_handle.asset_resolver().get("progress.html".to_string()) {
                        Some(asset) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", "text/html")
                            .body(asset.bytes)
                            .unwrap(),
                        None => {
                            let err_msg = "Failed to read progress.html: Not found in assets".to_string();
                            tauri::http::Response::builder()
                                .status(500)
                                .header("Content-Type", "text/plain")
                                .body(err_msg.into_bytes())
                                .unwrap()
                        }
                    };
                    responder.respond(response);
                    return;
                }

                // 2. Serving assets
                if is_asset {
                    let key = original_path.trim_start_matches('/');
                    let response = match app_handle.asset_resolver().get(key.to_string()) {
                        Some(asset) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", asset.mime_type)
                            .body(asset.bytes)
                            .unwrap(),
                        None => tauri::http::Response::builder()
                            .status(404)
                            .body(vec![])
                            .unwrap(),
                    };
                    responder.respond(response);
                    return;
                }

                // 3. Serving progress API
                if path == "/__progress_api__" {
                    let is_importing = import_state.is_importing.load(Ordering::Relaxed);
                    let phase_val = import_state.phase.load(Ordering::Relaxed);
                    let phase = match phase_val {
                        0 => "idle",
                        1 => "decoding",
                        2 => "importing",
                        3 => "done",
                        4 => "error",
                        _ => "unknown",
                    };
                    let processed_bytes = import_state.processed_bytes.load(Ordering::Relaxed);
                    let total_bytes = import_state.total_bytes.load(Ordering::Relaxed);
                    let processed_entries = import_state.processed_entries.load(Ordering::Relaxed);

                    let json = format!(
                        r#"{{"is_importing":{},"phase":"{}","processed_bytes":{},"total_bytes":{},"processed_entries":{}}}"#,
                        is_importing, phase, processed_bytes, total_bytes, processed_entries
                    );
                    let response = tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", "application/json")
                        .body(json.into_bytes())
                        .unwrap();
                    responder.respond(response);
                    return;
                }

                // 4. Serving content from local store (background thread owns the store)
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if host == "home" || host == "localhost" || host.is_empty() {
                        // 4a. Serve frontpage
                        let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                        if tx.send(StoreMsg::Frontpage {
                            order: path.to_string(),
                            resp: resp_tx,
                        }).is_err() {
                            return tauri::http::Response::builder()
                                .status(500)
                                .body(b"Store thread died".to_vec()).unwrap();
                        }
                        match resp_rx.recv().unwrap() {
                            Ok(html) => {
                                let body = rewrite_urls(html.as_bytes(), "text/html; charset=utf-8");
                                tauri::http::Response::builder()
                                    .status(200)
                                    .header("Content-Type", "text/html; charset=utf-8")
                                    .header("Access-Control-Allow-Origin", "*")
                                    .body(body)
                                    .unwrap()
                            }
                            Err(e) => {
                                tauri::http::Response::builder()
                                    .status(500)
                                    .body(format!("Frontpage error: {e}").into_bytes())
                                    .unwrap()
                            }
                        }
                    } else if host.len() == 64 {
                        // 4b. Serve subspace content
                        let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                        if tx.send(StoreMsg::GetEntry {
                            host: host.to_string(),
                            path: path.to_string(),
                            resp: resp_tx,
                        }).is_err() {
                            return tauri::http::Response::builder()
                                .status(500)
                                .body(b"Store thread died".to_vec()).unwrap();
                        }
                        match resp_rx.recv().unwrap() {
                            Ok(Some((mime, bytes, etag))) => {
                                let body = rewrite_urls(&bytes, &mime);
                                tauri::http::Response::builder()
                                    .status(200)
                                    .header("Content-Type", &mime)
                                    .header("ETag", etag)
                                    .header("Access-Control-Allow-Origin", "*")
                                    .body(body).unwrap()
                            }
                            Ok(None) => {
                                tauri::http::Response::builder()
                                    .status(404)
                                    .body(b"Not found".to_vec()).unwrap()
                            }
                            Err(e) => {
                                tauri::http::Response::builder()
                                    .status(500)
                                    .body(format!("Content error: {e}").into_bytes()).unwrap()
                            }
                        }
                    } else {
                        tauri::http::Response::builder()
                            .status(400)
                            .body(format!("Invalid host: {host}").into_bytes()).unwrap()
                    }
                }));
                let response = match result {
                    Ok(response) => response,
                    Err(panic) => {
                        let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "unknown panic".to_string()
                        };
                        tauri::http::Response::builder()
                            .status(500)
                            .body(format!("PANIC: {msg}").into_bytes())
                            .unwrap()
                    }
                };
                responder.respond(response);
            });
        })
        .setup(move |app| {
            let window = tauri::window::WindowBuilder::new(app, "main")
                .title("walky6")
                .inner_size(1024.0, 768.0)
                .resizable(true)
                .fullscreen(false)
                .build()
                .expect("failed to create main window");

            let home_url: url::Url = "sneaker://home/".parse().unwrap();

            // Log when any webview-navigated event is emitted/received in Rust
            {
                app.listen("webview-navigated", move |event| {
                    println!("[Rust setup listen] Received webview-navigated event in Rust: {:?}", event.payload());
                });
            }

            let scale_factor = window.scale_factor().unwrap_or(1.0);
            let win_size = window.inner_size().unwrap_or(tauri::PhysicalSize::new(1024, 768));
            let toolbar_h_physical = (TOOLBAR_H as f64 * scale_factor).round() as u32;
            let content_h = win_size.height.saturating_sub(toolbar_h_physical);

            let _main_webview = window
                .add_child(
                    tauri::webview::WebviewBuilder::new("main", tauri::WebviewUrl::App("index.html".into())),
                    tauri::PhysicalPosition::new(0u32, 0u32),
                    tauri::PhysicalSize::new(win_size.width, toolbar_h_physical),
                )
                .expect("failed to create main webview");

            #[cfg(not(target_os = "linux"))]
            let _ = _main_webview.set_auto_resize(false);

            let builder =
                tauri::webview::WebviewBuilder::new("content", tauri::WebviewUrl::CustomProtocol(home_url.clone()))
                    .initialization_script(r#"
                        (function() {
                            console.log("[Content JS] Init script starting. top-level:", window === window.top, "URL:", location.href);
                            if (window !== window.top) return;
                            function normalizeUrl(url) {
                                if (url.indexOf("http://sneaker.localhost/") === 0) {
                                    var rest = url.substring("http://sneaker.localhost/".length);
                                    var slash = rest.indexOf("/");
                                    if (slash === -1) return "sneaker://" + rest + "/";
                                    return "sneaker://" + rest.substring(0, slash) + rest.substring(slash);
                                }
                                return url;
                            }
                            function emit(url, action) {
                                console.log("[Content JS] emit called. url:", url, "action:", action);
                                url = normalizeUrl(url);
                                console.log("[Content JS] normalized url:", url);
                                try {
                                    if (window.__TAURI_INTERNALS__) {
                                        console.log("[Content JS] window.__TAURI_INTERNALS__ is available");
                                        if (window.__TAURI_INTERNALS__.invoke) {
                                            console.log("[Content JS] invoking webview_navigated command");
                                            window.__TAURI_INTERNALS__.invoke("webview_navigated", { url: url, action: action })
                                                .then(function() {
                                                    console.log("[Content JS] webview_navigated command completed successfully");
                                                })
                                                .catch(function(err) {
                                                    console.error("[Content JS] webview_navigated command failed:", err);
                                                });
                                        } else {
                                            console.warn("[Content JS] window.__TAURI_INTERNALS__.invoke is undefined");
                                        }
                                        
                                        if (window.__TAURI_INTERNALS__.emit) {
                                            console.log("[Content JS] calling window.__TAURI_INTERNALS__.emit");
                                            window.__TAURI_INTERNALS__.emit("webview-navigated", { url: url, action: action });
                                        } else {
                                            console.log("[Content JS] window.__TAURI_INTERNALS__.emit is undefined");
                                        }
                                    } else {
                                        console.warn("[Content JS] window.__TAURI_INTERNALS__ is undefined");
                                    }

                                    if (window.__TAURI__ && window.__TAURI__.event && window.__TAURI__.event.emit) {
                                        console.log("[Content JS] calling window.__TAURI__.event.emit");
                                        window.__TAURI__.event.emit("webview-navigated", { url: url, action: action });
                                    }
                                } catch(e) {
                                    console.error("[Content JS] Exception in emit:", e);
                                }
                            }
                            var _push = history.pushState;
                            history.pushState = function() { _push.apply(this, arguments); emit(location.href, "push"); };
                            var _replace = history.replaceState;
                            history.replaceState = function() { _replace.apply(this, arguments); emit(location.href, "replace"); };
                            window.addEventListener("popstate", function() { emit(location.href, "pop"); });
                            window.addEventListener("hashchange", function() { emit(location.href, "pop"); });
                            window.addEventListener("load", function() { emit(location.href, "load"); });
                            if (document.readyState === "complete") {
                                emit(location.href, "load");
                            }
                            document.addEventListener("click", function(e) {
                                var a = e.target.closest("a");
                                if (a && a.href && a.href.indexOf("sneaker://") === 0) {
                                    e.preventDefault();
                                    console.log("[Content JS] Intercepted sneaker:// link click:", a.href);
                                    try {
                                        if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
                                            console.log("[Content JS] invoking navigate_content for:", a.href);
                                            window.__TAURI_INTERNALS__.invoke("navigate_content", { path: a.href })
                                                .catch(function(err) {
                                                    console.error("[Content JS] navigate_content failed:", err);
                                                });
                                        } else {
                                            console.warn("[Content JS] cannot invoke navigate_content, internals or invoke missing");
                                        }
                                    } catch(err) {
                                        console.error("[Content JS] Exception in click intercept:", err);
                                    }
                                }
                            });
                        })();
                    "#)
                    .on_navigation(move |_url| {
                        true
                    })
                    .on_new_window({
                        let app_handle = app.app_handle().clone();
                        move |url, _features| {
                            let state = app_handle.state::<ContentWebview>();
                            if let Ok(guard) = state.0.lock() {
                                if let Some(ref wv) = *guard {
                                    let target_url = url.clone();
                                    #[cfg(target_os = "windows")]
                                    let target_url = {
                                        if target_url.scheme() == "sneaker" {
                                            let host = target_url.host_str().unwrap_or("home");
                                            let path_and_query = match target_url.query() {
                                                Some(q) => format!("{}?{}", target_url.path(), q),
                                                None => target_url.path().to_string(),
                                            };
                                            if let Ok(u) = url::Url::parse(&format!("http://sneaker.localhost/{}{}", host, path_and_query)) {
                                                u
                                            } else {
                                                target_url
                                            }
                                        } else {
                                            target_url
                                        }
                                    };
                                    let _ = wv.navigate(target_url);
                                }
                            }
                            tauri::webview::NewWindowResponse::Deny
                        }
                    });

            let webview = window
                .add_child(
                    builder,
                    tauri::PhysicalPosition::new(0u32, toolbar_h_physical),
                    tauri::PhysicalSize::new(win_size.width, content_h),
                )
                .expect("failed to add content webview");

            #[cfg(not(target_os = "linux"))]
            let _ = webview.set_auto_resize(false);

            {
                let state = app.state::<ContentWebview>();
                *state.0.lock().unwrap() = Some(webview);
            }

            #[cfg(target_os = "linux")]
            fix_linux_webview_packing(&window);

            #[cfg(not(target_os = "linux"))]
            {
                let wv_arc = app.state::<ContentWebview>().0.clone();
                let main_wv = _main_webview.clone();
                let window_clone = window.clone();
                window.on_window_event(move |event| {
                    let trigger = match event {
                        tauri::WindowEvent::Resized(_) => true,
                        tauri::WindowEvent::ScaleFactorChanged { .. } => true,
                        _ => false,
                    };
                    if trigger {
                        let scale_factor = window_clone.scale_factor().unwrap_or(1.0);
                        let toolbar_h_physical = (TOOLBAR_H as f64 * scale_factor).round() as u32;
                        if let Ok(size) = window_clone.inner_size() {
                            let _ = main_wv.set_position(tauri::PhysicalPosition::new(0u32, 0u32));
                            let _ = main_wv.set_size(tauri::PhysicalSize::new(size.width, toolbar_h_physical));
                            if let Some(ref wv) = *wv_arc.lock().unwrap() {
                                let _ = wv.set_position(tauri::PhysicalPosition::new(0u32, toolbar_h_physical));
                                let _ = wv.set_size(tauri::PhysicalSize::new(
                                    size.width,
                                    size.height.saturating_sub(toolbar_h_physical),
                                ));
                            }
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_base_url,
            navigate_content,
            import_file,
            pick_file,
            go_back_content,
            go_forward_content,
            webview_navigated
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(target_os = "linux")]
fn fix_linux_webview_packing(window: &tauri::window::Window) {
    use gtk::prelude::*;
    if let Ok(gtk_win) = window.gtk_window() {
        gtk_win.set_titlebar(Option::<&gtk::Widget>::None);
        if let Some(container) = gtk_win.child() {
            if let Ok(container_box) = container.downcast::<gtk::Box>() {
                let children = container_box.children();
                for (index, child) in children.iter().enumerate() {
                    if index == 0 {
                        // Set explicit size request for the toolbar to prevent it from collapsing to 0 height
                        child.set_size_request(-1, TOOLBAR_H as i32);
                        // Toolbar webview: expand = false, fill = false
                        container_box.set_child_packing(child, false, false, 0, gtk::PackType::Start);
                    } else if index == 1 {
                        // Content webview: expand = true, fill = true
                        container_box.set_child_packing(child, true, true, 0, gtk::PackType::Start);
                    }
                }
            }
        }
    }
}
