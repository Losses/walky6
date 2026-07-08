use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

const TOOLBAR_H: u32 = 90;

enum StoreMsg {
    Frontpage { order: String, resp: std::sync::mpsc::Sender<anyhow::Result<String>> },
    GetEntry { host: String, path: String, resp: std::sync::mpsc::Sender<anyhow::Result<Option<(String, Vec<u8>, String)>>> },
    Import {
        file_path: PathBuf,
        handle: sneakerweb::import::ProgressHandle,
        resp: std::sync::mpsc::Sender<anyhow::Result<()>>,
    },
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
                StoreMsg::Import { file_path, handle, resp } => {
                    let args = sneakerweb::import::ImportArgs {
                        src: file_path,
                        mode: None,
                    };
                    let result = smol::block_on(
                        sneakerweb::import::import_sneak_into_store(&args, &handle, &mut store),
                    );
                    let _ = resp.send(result);
                }
            }
        }
    });
    tx
}

pub struct StoreTx(pub(crate) std::sync::mpsc::Sender<StoreMsg>);

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

#[cfg(target_os = "windows")]
fn extract_hash_from_referer(referer: &str) -> Option<String> {
    eprintln!("[extract_hash] checking referer: {}", referer);
    static RE: std::sync::OnceLock<regex_lite::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex_lite::Regex::new(r#"^https?://sneaker\.localhost(?:[:/]|$)"#).expect("failed to compile referer regex")
    });
    if !re.is_match(referer) {
        eprintln!("[extract_hash] referer does not match sneaker.localhost pattern");
        return None;
    }
    let after_host = if referer.starts_with("http://") {
        &referer["http://sneaker.localhost".len()..]
    } else if referer.starts_with("https://") {
        &referer["https://sneaker.localhost".len()..]
    } else {
        eprintln!("[extract_hash] referer does not start with http:// or https://");
        return None;
    };
    eprintln!("[extract_hash] after_host: {}", after_host);
    let path_part = after_host.trim_start_matches(':').trim_start_matches('/');
    let first_segment = path_part.split('/').next().unwrap_or("");
    eprintln!("[extract_hash] first_segment: {} (len={})", first_segment, first_segment.len());
    if first_segment.len() == 64 && first_segment.chars().all(|c| c.is_ascii_hexdigit()) {
        eprintln!("[extract_hash] extracted hash: {}", first_segment);
        Some(first_segment.to_string())
    } else {
        eprintln!("[extract_hash] first_segment is not a valid hash");
        None
    }
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
            { format!("http://home.localhost{path}") }
            #[cfg(not(target_os = "windows"))]
            { format!("sneaker://home{path}") }
        } else {
            #[cfg(target_os = "windows")]
            { format!("http://{subdomain}.localhost{path}") }
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

fn run_import_internal(app: AppHandle, file_path: PathBuf, import_state: Arc<ImportState>, store_tx: std::sync::mpsc::Sender<StoreMsg>) -> anyhow::Result<()> {
    import_state.is_importing.store(true, Ordering::Relaxed);
    import_state.phase.store(1, Ordering::Relaxed);
    import_state.processed_bytes.store(0, Ordering::Relaxed);
    import_state.processed_entries.store(0, Ordering::Relaxed);

    let handle = sneakerweb::import::ProgressHandle::new();
    import_state.total_bytes.store(0, Ordering::Relaxed);

    let poller_handle = handle.clone();
    let signal_handle = handle.clone();
    let app_clone = app.clone();
    let state_clone = import_state.clone();

    let poller = std::thread::spawn(move || {
        loop {
            let progress = progress_to_event(&poller_handle);

            if progress.total > 0 {
                state_clone.total_bytes.store(progress.total, Ordering::Relaxed);
            }
            state_clone.phase.store(poller_handle.phase.load(Ordering::Relaxed), Ordering::Relaxed);
            state_clone.processed_bytes.store(progress.processed, Ordering::Relaxed);
            state_clone.processed_entries.store(progress.processed_entries, Ordering::Relaxed);

            let is_done = progress.phase == "done";
            let _ = app_clone.emit("import-progress", progress.clone());
            if is_done {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    });

    let (resp_tx, resp_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    store_tx
        .send(StoreMsg::Import {
            file_path,
            handle,
            resp: resp_tx,
        })
        .map_err(|_| anyhow::anyhow!("store thread died"))?;

    let import_result = resp_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("store thread channel closed"))?;
    // import_result is anyhow::Result<()> from import_sneak_into_store

    // Signal poller to stop (in case import errored without setting PHASE_DONE)
    signal_handle.phase.store(sneakerweb::import::PHASE_DONE, Ordering::Relaxed);

    let _ = poller.join();

    let result = match &import_result {
        Ok(()) => {
            import_state.phase.store(3, Ordering::Relaxed);
            Ok(())
        }
        Err(e) => {
            import_state.phase.store(4, Ordering::Relaxed);
            Err(anyhow::anyhow!("Import failed: {}", e))
        }
    };

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
    app.emit("webview-navigated", NavPayload { url, action })
        .map_err(|e| format!("Failed to emit webview-navigated: {e}"))
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
        let combined = format!("{base}{clean_path}");
        url::Url::parse(&combined).map_err(|e| format!("invalid URL: {e}"))?
    };

    #[cfg(target_os = "windows")]
    let parsed = if parsed.scheme() == "sneaker" {
        let host = parsed.host_str().unwrap_or("home");
        let path_and_query = match parsed.query() {
            Some(q) => format!("{}?{}", parsed.path(), q),
            None => parsed.path().to_string(),
        };
        let rewritten = url::Url::parse(&format!("http://{}.localhost{}", host, path_and_query))
            .map_err(|e| e.to_string())?;
        rewritten
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
async fn clear_content(content_wv: tauri::State<'_, ContentWebview>) -> Result<(), String> {
    let wv = content_wv.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = *wv {
        wv.eval("document.open(); document.close();").map_err(|e| e.to_string())?;
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
async fn import_file(app: AppHandle, file_path: String, import_state: tauri::State<'_, Arc<ImportState>>, store_tx: tauri::State<'_, StoreTx>) -> Result<(), String> {
    let path = PathBuf::from(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {file_path}"));
    }
    run_import_internal(app, path, import_state.inner().clone(), store_tx.0.clone()).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
mod webview2_handler {
    use super::*;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use webview2_com::WebResourceRequestedEventHandler;
    use windows::core::{HSTRING, PCWSTR, Interface};
    use windows::Win32::UI::Shell::SHCreateMemStream;
    use std::fmt::Write;

    type EventRegistrationToken = i64;

    pub fn register_wildcard_localhost_handler(
        webview: &tauri::Webview,
        store_tx: std::sync::mpsc::Sender<StoreMsg>,
        import_state: Arc<ImportState>,
    ) -> Result<(), String> {
        let app_handle = webview.app_handle().clone();
        webview.with_webview(move |wv| {
            let controller = wv.controller();
            let core_webview = unsafe { controller.CoreWebView2() }.expect("failed to get CoreWebView2");
            let env = wv.environment();

            let filter = HSTRING::from("http://*.localhost/*");
            
            unsafe {
                let result = core_webview.cast::<ICoreWebView2_22>();
                match result {
                    Ok(core_webview_22) => {
                        eprintln!("[webview2-handler] using ICoreWebView2_22 for cross-origin iframe support");
                        core_webview_22.AddWebResourceRequestedFilterWithRequestSourceKinds(
                            &filter,
                            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                            COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
                        ).expect("failed to add filter with request source kinds");
                    }
                    Err(_) => {
                        eprintln!("[webview2-handler] ICoreWebView2_22 not available, falling back to legacy API (cross-origin iframes may not work)");
                        core_webview.AddWebResourceRequestedFilter(
                            &filter,
                            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                        ).expect("failed to add filter");
                    }
                }
            }

            let store_tx_clone = store_tx.clone();
            let env_clone = env.clone();
            let app_handle_clone = app_handle.clone();
            let import_state_clone = import_state.clone();

            let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()); };

                let request = unsafe { args.Request() }.expect("failed to get request");
                let mut uri_pwstr = windows::core::PWSTR::null();
                unsafe { request.Uri(&mut uri_pwstr) }.expect("failed to get uri");
                let uri = unsafe { windows::core::HSTRING::from_wide(uri_pwstr.as_wide()) };
                let uri_str = uri.to_string();

                eprintln!("[webview2-handler] request: {}", uri_str);

                let parsed = match url::Url::parse(&uri_str) {
                    Ok(u) => u,
                    Err(_) => return Ok(()),
                };

                if parsed.scheme() != "http" {
                    return Ok(());
                }

                let host = parsed.host_str().unwrap_or("");
                if !host.ends_with(".localhost") {
                    return Ok(());
                }

                let subdomain = &host[..host.len() - ".localhost".len()];
                let path = parsed.path().to_string();

                eprintln!("[webview2-handler] host={}, path={}", subdomain, path);

                let is_subspace = subdomain.len() == 64 && subdomain.chars().all(|c| c.is_ascii_hexdigit());
                let is_frontpage = subdomain.is_empty() || subdomain == "sneaker" || subdomain == "home";
                let is_ipc = subdomain == "ipc";
                let is_nav = subdomain == "__nav__";

                if is_nav {
                    let mut nav_url = String::new();
                    let mut nav_action = String::from("push");
                    if let Some(query) = parsed.query() {
                        for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
                            match k.as_ref() {
                                "url" => nav_url = v.into_owned(),
                                "action" => nav_action = v.into_owned(),
                                _ => {}
                            }
                        }
                    }
                    let _ = app_handle_clone.emit("webview-navigated", NavPayload { url: nav_url, action: nav_action });
                    let stream = unsafe { SHCreateMemStream(Some(&[])) };
                    let status = HSTRING::from("OK");
                    let headers = HSTRING::from("Access-Control-Allow-Origin: *\r\n");
                    let response = unsafe {
                        env_clone.CreateWebResourceResponse(stream.as_ref(), 200, PCWSTR::from_raw(status.as_ptr()), PCWSTR::from_raw(headers.as_ptr()))
                    };
                    if let Ok(resp) = response {
                        let _ = unsafe { args.SetResponse(&resp) };
                    }
                    return Ok(());
                }

                if is_ipc {
                    return Ok(());
                }

                if is_frontpage && path == "/__progress__" {
                    let response_body = match app_handle_clone.asset_resolver().get("progress.html".to_string()) {
                        Some(asset) => asset.bytes,
                        None => {
                            let stream = unsafe { SHCreateMemStream(Some(&[])) };
                            let status = HSTRING::from("Not Found");
                            let headers = HSTRING::from("");
                            let response = unsafe {
                                env_clone.CreateWebResourceResponse(
                                    stream.as_ref(),
                                    404,
                                    PCWSTR::from_raw(status.as_ptr()),
                                    PCWSTR::from_raw(headers.as_ptr()),
                                )
                            };
                            if let Ok(resp) = response {
                                let _ = unsafe { args.SetResponse(&resp) };
                            }
                            return Ok(());
                        }
                    };
                    let stream = unsafe { SHCreateMemStream(Some(&response_body)) };
                    let status = HSTRING::from("OK");
                    let headers = HSTRING::from("Content-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\n");
                    let response = unsafe {
                        env_clone.CreateWebResourceResponse(
                            stream.as_ref(),
                            200,
                            PCWSTR::from_raw(status.as_ptr()),
                            PCWSTR::from_raw(headers.as_ptr()),
                        )
                    };
                    if let Ok(resp) = response {
                        let _ = unsafe { args.SetResponse(&resp) };
                    }
                    return Ok(());
                }

                if is_frontpage && path == "/__progress_api__" {
                    let is_importing = import_state_clone.is_importing.load(Ordering::Relaxed);
                    let phase_val = import_state_clone.phase.load(Ordering::Relaxed);
                    let phase = match phase_val {
                        0 => "idle",
                        1 => "decoding",
                        2 => "importing",
                        3 => "done",
                        4 => "error",
                        _ => "unknown",
                    };
                    let processed_bytes = import_state_clone.processed_bytes.load(Ordering::Relaxed);
                    let total_bytes = import_state_clone.total_bytes.load(Ordering::Relaxed);
                    let processed_entries = import_state_clone.processed_entries.load(Ordering::Relaxed);

                    let json = format!(
                        r#"{{"is_importing":{},"phase":"{}","processed_bytes":{},"total_bytes":{},"processed_entries":{}}}"#,
                        is_importing, phase, processed_bytes, total_bytes, processed_entries
                    );
                    let stream = unsafe { SHCreateMemStream(Some(json.as_bytes())) };
                    let status = HSTRING::from("OK");
                    let headers = HSTRING::from("Content-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n");
                    let response = unsafe {
                        env_clone.CreateWebResourceResponse(
                            stream.as_ref(),
                            200,
                            PCWSTR::from_raw(status.as_ptr()),
                            PCWSTR::from_raw(headers.as_ptr()),
                        )
                    };
                    if let Ok(resp) = response {
                        let _ = unsafe { args.SetResponse(&resp) };
                    }
                    return Ok(());
                }

                if is_frontpage {
                    let is_static_asset = {
                        let lower = path.to_lowercase();
                        lower.ends_with(".js") || lower.ends_with(".mjs")
                            || lower.ends_with(".css") || lower.ends_with(".map")
                            || lower.ends_with(".png") || lower.ends_with(".jpg")
                            || lower.ends_with(".jpeg") || lower.ends_with(".webp")
                            || lower.ends_with(".gif") || lower.ends_with(".svg")
                            || lower.ends_with(".ico") || lower.ends_with(".avif")
                            || lower.ends_with(".woff") || lower.ends_with(".woff2")
                            || lower.ends_with(".ttf") || lower.ends_with(".otf")
                    };
                    let is_asset = is_static_asset
                        || path.starts_with("/_app/")
                        || path.starts_with("/assets/")
                        || path.starts_with("/images/")
                        || path.starts_with("/fonts/");

                    if is_asset {
                        let key = path.trim_start_matches('/');
                        match app_handle_clone.asset_resolver().get(key.to_string()) {
                            Some(asset) => {
                                let stream = unsafe { SHCreateMemStream(Some(&asset.bytes)) };
                                let status = HSTRING::from("OK");
                                let headers = HSTRING::from(format!("Content-Type: {}\r\nAccess-Control-Allow-Origin: *\r\n", asset.mime_type));
                                let response = unsafe {
                                    env_clone.CreateWebResourceResponse(
                                        stream.as_ref(),
                                        200,
                                        PCWSTR::from_raw(status.as_ptr()),
                                        PCWSTR::from_raw(headers.as_ptr()),
                                    )
                                };
                                if let Ok(resp) = response {
                                    let _ = unsafe { args.SetResponse(&resp) };
                                }
                            }
                            None => {
                                let stream = unsafe { SHCreateMemStream(Some(&[])) };
                                let status = HSTRING::from("Not Found");
                                let headers = HSTRING::from("");
                                let response = unsafe {
                                    env_clone.CreateWebResourceResponse(
                                        stream.as_ref(),
                                        404,
                                        PCWSTR::from_raw(status.as_ptr()),
                                        PCWSTR::from_raw(headers.as_ptr()),
                                    )
                                };
                                if let Ok(resp) = response {
                                    let _ = unsafe { args.SetResponse(&resp) };
                                }
                            }
                        }
                        return Ok(());
                    }
                }

                if is_frontpage {
                    let deferral = unsafe { args.GetDeferral() };
                    let store_tx_inner = store_tx_clone.clone();
                    let env_inner = env_clone.clone();
                    let path_owned = path.to_string();

                    let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                    if store_tx_inner.send(StoreMsg::Frontpage {
                        order: path_owned,
                        resp: resp_tx,
                    }).is_err() {
                        return Ok(());
                    }

                    match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                        Ok(Ok(html)) => {
                            let body = rewrite_urls(html.as_bytes(), "text/html; charset=utf-8");
                            let stream = unsafe { SHCreateMemStream(Some(&body)) };
                            let status = HSTRING::from("OK");
                            let headers = HSTRING::from("Content-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\n");

                            let response = unsafe {
                                env_inner.CreateWebResourceResponse(
                                    stream.as_ref(),
                                    200,
                                    PCWSTR::from_raw(status.as_ptr()),
                                    PCWSTR::from_raw(headers.as_ptr()),
                                )
                            };
                            if let Ok(resp) = response {
                                let _ = unsafe { args.SetResponse(&resp) };
                            }
                        }
                        _ => {
                            let stream = unsafe { SHCreateMemStream(Some(&[])) };
                            let status = HSTRING::from("Service Unavailable");
                            let headers = HSTRING::from("Retry-After: 5\r\nAccess-Control-Allow-Origin: *\r\n");
                            let response = unsafe {
                                env_inner.CreateWebResourceResponse(
                                    stream.as_ref(),
                                    503,
                                    PCWSTR::from_raw(status.as_ptr()),
                                    PCWSTR::from_raw(headers.as_ptr()),
                                )
                            };
                            if let Ok(resp) = response {
                                let _ = unsafe { args.SetResponse(&resp) };
                            }
                        }
                    }

                    if let Ok(def) = &deferral {
                        let _ = unsafe { def.Complete() };
                    }
                    return Ok(());
                }

                if is_subspace {
                    let deferral = unsafe { args.GetDeferral() };
                    let store_tx_inner = store_tx_clone.clone();
                    let env_inner = env_clone.clone();
                    let subdomain_owned = subdomain.to_string();
                    let path_owned = path.to_string();

                    // Do the store lookup synchronously (we're already on a background thread)
                    let try_paths: Vec<String> = if path_owned.ends_with('/') {
                        vec![path_owned.clone(), format!("{}index.html", path_owned)]
                    } else {
                        vec![
                            path_owned.clone(),
                            format!("{}.html", path_owned),
                            format!("{}/index.html", path_owned),
                        ]
                    };

                    let mut found: Option<(String, Vec<u8>, String)> = None;

                    for candidate in &try_paths {
                        let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                        if store_tx_inner.send(StoreMsg::GetEntry {
                            host: subdomain_owned.clone(),
                            path: candidate.clone(),
                            resp: resp_tx,
                        }).is_err() {
                            break;
                        }
                        match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                            Ok(Ok(Some((mime, bytes, etag)))) => {
                                found = Some((mime, bytes, etag));
                                break;
                            }
                            Ok(Ok(None)) => {}
                            Ok(Err(_)) | Err(_) => break,
                        }
                    }

                    if let Some((mime, bytes, etag)) = found {
                        let body = rewrite_urls(&bytes, &mime);
                        let stream = unsafe { SHCreateMemStream(Some(&body)) };
                        let status = HSTRING::from("OK");
                        let mut headers_map = String::new();
                        let _ = writeln!(headers_map, "Content-Type: {}", mime);
                        let _ = writeln!(headers_map, "ETag: {}", etag);
                        let _ = writeln!(headers_map, "Access-Control-Allow-Origin: *");
                        let headers = HSTRING::from(headers_map);

                        let response = unsafe {
                            env_inner.CreateWebResourceResponse(
                                stream.as_ref(),
                                200,
                                PCWSTR::from_raw(status.as_ptr()),
                                PCWSTR::from_raw(headers.as_ptr()),
                            )
                        };

                        if let Ok(resp) = response {
                            let _ = unsafe { args.SetResponse(&resp) };
                        }
                    } else {
                        let stream = unsafe { SHCreateMemStream(Some(&[])) };
                        let status = HSTRING::from("Not Found");
                        let headers = HSTRING::from("");

                        let response = unsafe {
                            env_inner.CreateWebResourceResponse(
                                stream.as_ref(),
                                404,
                                PCWSTR::from_raw(status.as_ptr()),
                                PCWSTR::from_raw(headers.as_ptr()),
                            )
                        };
                        if let Ok(resp) = response {
                            let _ = unsafe { args.SetResponse(&resp) };
                        }
                    }

                    if let Ok(def) = &deferral {
                        let _ = unsafe { def.Complete() };
                    }
                }

                Ok(())
            }));

            let mut token: EventRegistrationToken = 0;
            unsafe {
                core_webview.add_WebResourceRequested(&handler, &mut token)
            }.expect("failed to add handler");

            eprintln!("[webview2-handler] registered wildcard http://*.localhost/* handler");
        }).map_err(|e| e.to_string())?;

        Ok(())
    }
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

    let store_tx_for_manage = store_tx.clone();
    let store_tx_for_protocol = store_tx;
    let protocol_import_state = import_state.clone();
    let import_state_for_setup = import_state.clone();
    let store_tx_for_setup = store_tx_for_manage.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(import_state)
        .manage(ContentWebview(std::sync::Arc::new(Mutex::new(None))))
        .manage(StoreTx(store_tx_for_manage))
        .register_asynchronous_uri_scheme_protocol("sneaker", move |ctx, req, responder| {
            let import_state = protocol_import_state.clone();
            let app_handle = ctx.app_handle().clone();

            let tx = store_tx_for_protocol.clone();
            std::thread::spawn(move || {
                let host_buf = req.uri().host().unwrap_or("home").to_string();
                let path_str = req.uri().path().to_string();

                if host_buf == "__nav__" {
                    let mut nav_url = String::new();
                    let mut nav_action = String::from("push");
                    if let Some(query) = req.uri().query() {
                        for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
                            match k.as_ref() {
                                "url" => nav_url = v.into_owned(),
                                "action" => nav_action = v.into_owned(),
                                _ => {}
                            }
                        }
                    }
                    let _ = app_handle.emit("webview-navigated", NavPayload { url: nav_url, action: nav_action });
                    let response = tauri::http::Response::builder()
                        .status(200)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(Vec::new())
                        .unwrap();
                    responder.respond(response);
                    return;
                }

                let original_path = path_str.clone();

                let is_vite_dev = path_str.starts_with("/@vite/")
                    || path_str.starts_with("/@id/")
                    || path_str.starts_with("/@fs/")
                    || path_str.starts_with("/src/")
                    || path_str.starts_with("/node_modules/")
                    || path_str == "/@react-refresh";

                let is_static_asset = {
                    let lower = path_str.to_lowercase();
                    lower.ends_with(".js") || lower.ends_with(".mjs")
                        || lower.ends_with(".css") || lower.ends_with(".map")
                        || lower.ends_with(".png") || lower.ends_with(".jpg")
                        || lower.ends_with(".jpeg") || lower.ends_with(".webp")
                        || lower.ends_with(".gif") || lower.ends_with(".svg")
                        || lower.ends_with(".ico") || lower.ends_with(".avif")
                        || lower.ends_with(".woff") || lower.ends_with(".woff2")
                        || lower.ends_with(".ttf") || lower.ends_with(".otf")
                        || lower.ends_with(".mp4") || lower.ends_with(".webm")
                        || lower.ends_with(".mp3") || lower.ends_with(".ogg")
                        || lower.ends_with(".json") || lower.ends_with(".xml")
                };

                let is_asset = is_vite_dev
                    || is_static_asset
                    || path_str.starts_with("/_app/")
                    || path_str.starts_with("/assets/");

                #[cfg(target_os = "windows")]
                let (host_buf, path_str) = {
                    let mut host = host_buf.clone();
                    let mut path = path_str.clone();

                    eprintln!("[sneaker-protocol] initial: host={}, path={}, uri={}, is_asset={}", host, path, req.uri(), is_asset);

                    let is_special_path = is_asset || path == "/__progress__" || path == "/__progress_api__";

                    if host == "localhost" && !is_special_path {
                        let key = path.trim_start_matches('/');
                        let first_segment = key.split('/').next().unwrap_or("");
                        let path_has_hash_prefix = first_segment.len() == 64 
                            && first_segment.chars().all(|c| c.is_ascii_hexdigit());

                        if path_has_hash_prefix {
                            if let Some((hash_part, rest)) = key.split_once('/') {
                                host = hash_part.to_string();
                                path = format!("/{}", rest);
                            } else {
                                host = key.to_string();
                                path = "/".to_string();
                            }
                            eprintln!("[sneaker-protocol] extracted from path: host={}, path={}", host, path);
                        } else {
                            let referer = req.headers().get("referer").and_then(|v| v.to_str().ok()).unwrap_or("");
                            eprintln!("[sneaker-protocol] no hash in path, checking referer: {}", referer);
                            if let Some(hash) = extract_hash_from_referer(referer) {
                                host = hash;
                                eprintln!("[sneaker-protocol] extracted from referer: host={}, path={}", host, path);
                            } else {
                                eprintln!("[sneaker-protocol] no hash in referer, keeping host={}", host);
                            }
                        }
                    }

                    (host, path)
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

                // 2. Serving progress API
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

                // 3. Determine context and serve
                let is_subspace = host.len() == 64;
                let is_frontpage = host == "home" || host == "localhost" || host.is_empty();

                eprintln!("[sneaker-protocol] routing: host={}, path={}, is_subspace={}, is_frontpage={}, is_asset={}", host, path, is_subspace, is_frontpage, is_asset);

                // 3a. Subspace: everything goes through the store
                if is_subspace {
                    eprintln!("[sneaker-protocol] serving subspace: host={}, path={}", host, path);
                    let host_clone = host.to_string();
                    let path_clone = path.to_string();
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let try_paths: Vec<String> = if path_clone.ends_with('/') {
                            vec![path_clone.clone(), format!("{}index.html", path_clone)]
                        } else {
                            vec![
                                path_clone.clone(),
                                format!("{}.html", path_clone),
                                format!("{}/index.html", path_clone),
                            ]
                        };

                        let mut found: Option<(String, Vec<u8>, String)> = None;

                        for candidate in &try_paths {
                            let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                            if tx.send(StoreMsg::GetEntry {
                                host: host_clone.clone(),
                                path: candidate.clone(),
                                resp: resp_tx,
                            }).is_err() {
                                return tauri::http::Response::builder()
                                    .status(500)
                                    .body(b"Store thread died".to_vec()).unwrap();
                            }
                            match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                                Ok(Ok(Some((mime, bytes, etag)))) => {
                                    found = Some((mime, bytes, etag));
                                    break;
                                }
                                Ok(Ok(None)) => {}
                                Ok(Err(e)) => {
                                    return tauri::http::Response::builder()
                                        .status(503)
                                        .body(format!("Store busy: {e}").into_bytes()).unwrap();
                                }
                                Err(_) => {
                                    return tauri::http::Response::builder()
                                        .status(503)
                                        .body(b"Store busy (timeout)".to_vec()).unwrap();
                                }
                            }
                        }

                        if let Some((mime, bytes, etag)) = found {
                            let body = rewrite_urls(&bytes, &mime);
                            tauri::http::Response::builder()
                                .status(200)
                                .header("Content-Type", &mime)
                                .header("ETag", etag)
                                .header("Access-Control-Allow-Origin", "*")
                                .body(body).unwrap()
                        } else {
                            tauri::http::Response::builder()
                                .status(404)
                                .body(b"Not found".to_vec()).unwrap()
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
                    return;
                }

                // 3b. Frontpage: serve static assets from asset resolver, everything else from store
                if is_frontpage && is_asset {
                    let key = original_path.trim_start_matches('/');
                    eprintln!("[sneaker-protocol] serving frontpage asset from resolver: {}", key);
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

                // 3c. Frontpage: serve HTML/SPA routes from store
                if is_frontpage {
                    eprintln!("[sneaker-protocol] serving frontpage from store: path={}", path);
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let (resp_tx, resp_rx) = std::sync::mpsc::channel();
                        if tx.send(StoreMsg::Frontpage {
                            order: path.to_string(),
                            resp: resp_tx,
                        }).is_err() {
                            return tauri::http::Response::builder()
                                .status(500)
                                .body(b"Store thread died".to_vec()).unwrap();
                        }
                        match resp_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                            Ok(Ok(html)) => {
                                let body = rewrite_urls(html.as_bytes(), "text/html; charset=utf-8");
                                tauri::http::Response::builder()
                                    .status(200)
                                    .header("Content-Type", "text/html; charset=utf-8")
                                    .header("Access-Control-Allow-Origin", "*")
                                    .body(body)
                                    .unwrap()
                            }
                            Ok(Err(e)) => {
                                tauri::http::Response::builder()
                                    .status(503)
                                    .body(format!("Store busy: {e}").into_bytes())
                                    .unwrap()
                            }
                            Err(_) => {
                                tauri::http::Response::builder()
                                    .status(503)
                                    .body(b"Store busy (timeout)".to_vec())
                                    .unwrap()
                            }
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
                    return;
                }

                // 3d. Unknown host
                eprintln!("[sneaker-protocol] invalid host: {}", host);
                let response = tauri::http::Response::builder()
                    .status(400)
                    .body(format!("Invalid host: {host}").into_bytes())
                    .unwrap();
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
                            if (window !== window.top) return;
                            
                            function normalizeUrl(url) {
                                var prefix = "http://";
                                var suffix = ".localhost";
                                if (url.indexOf(prefix) === 0) {
                                    var rest = url.substring(prefix.length);
                                    var dotIndex = rest.indexOf(suffix);
                                    if (dotIndex !== -1) {
                                        var host = rest.substring(0, dotIndex);
                                        var path = rest.substring(dotIndex + suffix.length);
                                        if (!path || path.length === 0) path = "/";
                                        return "sneaker://" + host + path;
                                    }
                                }
                                return url;
                            }
                            function emit(url, action) {
                                url = normalizeUrl(url);
                                var fetchUrl;
                                if (location.protocol === "http:" || location.protocol === "https:") {
                                    fetchUrl = "http://__nav__.localhost/?url=" + encodeURIComponent(url) + "&action=" + encodeURIComponent(action);
                                } else {
                                    fetchUrl = "sneaker://__nav__/?url=" + encodeURIComponent(url) + "&action=" + encodeURIComponent(action);
                                }
                                try { fetch(fetchUrl, { mode: "no-cors" }).catch(function(){}); } catch(e) {}
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
                                    try {
                                        if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
                                            window.__TAURI_INTERNALS__.invoke("navigate_content", { path: a.href }).catch(function() {});
                                        }
                                    } catch(err) {}
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
                                            if let Ok(u) = url::Url::parse(&format!("http://{}.localhost{}", host, path_and_query)) {
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

            #[cfg(target_os = "windows")]
            {
                let state = app.state::<ContentWebview>();
                let wv_clone = state.0.lock().unwrap().clone();
                if let Some(ref wv) = wv_clone {
                    let _ = webview2_handler::register_wildcard_localhost_handler(
                        wv,
                        store_tx_for_setup.clone(),
                        import_state_for_setup.clone(),
                    );
                }
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
            clear_content,
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
