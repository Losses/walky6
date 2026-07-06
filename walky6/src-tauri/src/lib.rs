use std::collections::HashMap;
use std::io::{BufRead, BufReader as StdBufReader, Read, Write};
use std::net::{TcpListener as StdTcpListener, TcpStream as StdTcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

const TOOLBAR_H: u32 = 90;

pub struct SneakerwebState {
    pub sneakerweb_port: u16,
    pub dir: PathBuf,
    pub dist_dir: PathBuf,
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

pub fn get_free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(8080)
}

pub fn ensure_sneakerweb_dir() -> PathBuf {
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

fn wait_for_port(port: u16, timeout_ms: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if StdTcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}

pub fn start_sneakerweb_server(port: u16, dir: PathBuf) {
    unsafe {
        std::env::set_var("SNEAKERWEB_DIR", &dir);
        std::env::set_var("PORT", port.to_string());
    }
    std::thread::spawn(move || {
        if let Err(e) = smol::block_on(sneakerweb::serve::start_server(port)) {
            eprintln!("sneakerweb server error: {e}");
        }
    });
}

fn forward_to_upstream(
    host: &str,
    port: u16,
    method: &str,
    path_and_query: &str,
    headers: &tauri::http::HeaderMap,
    body: &[u8],
) -> anyhow::Result<(u16, HashMap<String, String>, Vec<u8>)> {
    let mut stream = StdTcpStream::connect(format!("127.0.0.1:{port}"))?;
    let mut req_str = format!("{method} {path_and_query} HTTP/1.1\r\nHost: {host}\r\n");

    for (k, v) in headers {
        let kl = k.as_str().to_lowercase();
        if kl != "host" && kl != "connection" && kl != "proxy-connection" && kl != "accept-encoding" {
            if let Ok(v_str) = v.to_str() {
                req_str.push_str(&format!("{}: {}\r\n", k.as_str(), v_str));
            }
        }
    }
    req_str.push_str("Connection: close\r\n");

    if !body.is_empty() {
        req_str.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req_str.push_str("\r\n");

    stream.write_all(req_str.as_bytes())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()?;

    let mut reader = StdBufReader::new(stream.try_clone()?);
    let mut status_line = String::new();
    reader.read_line(&mut status_line)?;
    let status_parts: Vec<&str> = status_line.trim().split_whitespace().collect();
    let status_code: u16 = if status_parts.len() >= 2 {
        status_parts[1].parse().unwrap_or(500)
    } else {
        500
    };

    let mut resp_headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim().to_string();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            resp_headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }

    let mut resp_body = Vec::new();
    reader.read_to_end(&mut resp_body)?;

    Ok((status_code, resp_headers, resp_body))
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
    #[cfg(target_os = "windows")]
    { "http://sneaker.localhost/".to_string() }
    #[cfg(not(target_os = "windows"))]
    { "sneaker://localhost/".to_string() }
}

#[tauri::command]
async fn navigate_content(
    content_wv: tauri::State<'_, ContentWebview>,
    path: String,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let base = "http://sneaker.localhost";
    #[cfg(not(target_os = "windows"))]
    let base = "sneaker://localhost";

    let url = format!("{base}{path}");
    let parsed: url::Url = url.parse().map_err(|e| format!("invalid URL: {e}"))?;
    let wv = content_wv.0.lock().map_err(|e| e.to_string())?;
    if let Some(ref wv) = *wv {
        wv.navigate(parsed).map_err(|e| e.to_string())?;
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
        // Disable DMABUF renderer in WebKitGTK to prevent rendering artifacts/blurriness on Linux
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    let dist_dir = PathBuf::from(env!("WALKY6_DIST_DIR"));
    let import_state = ImportState::new();

    let sneakerweb_port = get_free_port();
    let dir = ensure_sneakerweb_dir();

    start_sneakerweb_server(sneakerweb_port, dir.clone());

    eprintln!("Waiting for sneakerweb server on port {sneakerweb_port}...");
    if !wait_for_port(sneakerweb_port, 5000) {
        eprintln!("sneakerweb server did not start in time");
    }

    let protocol_import_state = import_state.clone();
    let protocol_dist_dir = dist_dir.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(SneakerwebState {
            sneakerweb_port,
            dir,
            dist_dir: dist_dir.clone(),
        }))
        .manage(import_state)
        .manage(ContentWebview(std::sync::Arc::new(Mutex::new(None))))
        .register_asynchronous_uri_scheme_protocol("sneaker", move |_ctx, req, responder| {
            let import_state = protocol_import_state.clone();
            let dist_dir = protocol_dist_dir.clone();
            let sw_port = sneakerweb_port;

            std::thread::spawn(move || {
                let path = req.uri().path();
                let query = req.uri().query();

                // 1. Serving __progress__ page
                if path == "/__progress__" {
                    let progress_path = dist_dir.join("progress.html");
                    let response = match std::fs::read(&progress_path) {
                        Ok(content) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", "text/html")
                            .body(content)
                            .unwrap(),
                        Err(e) => {
                            let err_msg = format!("Failed to read progress.html: {e}");
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
                if path.starts_with("/assets/") {
                    let asset_path = dist_dir.join(path.trim_start_matches('/'));
                    if let Ok(content) = std::fs::read(&asset_path) {
                        let content_type = if path.ends_with(".js") {
                            "application/javascript"
                        } else if path.ends_with(".css") {
                            "text/css"
                        } else if path.ends_with(".svg") {
                            "image/svg+xml"
                        } else if path.ends_with(".woff2") {
                            "font/woff2"
                        } else if path.ends_with(".woff") {
                            "font/woff"
                        } else {
                            "application/octet-stream"
                        };
                        let response = tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", content_type)
                            .body(content)
                            .unwrap();
                        responder.respond(response);
                    } else {
                        let response = tauri::http::Response::builder()
                            .status(404)
                            .body(vec![])
                            .unwrap();
                        responder.respond(response);
                    }
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

                // 4. Serving proxy / upstream server requests
                let (upstream_host, upstream_path) = if path.starts_with("/proxy/") {
                    let rest = path.strip_prefix("/proxy/").unwrap_or("");
                    if let Some((domain, subpath)) = rest.split_once('/') {
                        let sp = if subpath.is_empty() { "/" } else { subpath };
                        let sp_formatted = if sp.starts_with('/') { sp.to_string() } else { format!("/{sp}") };
                        (format!("{domain}.localhost:{sw_port}"), sp_formatted)
                    } else {
                        (format!("{rest}.localhost:{sw_port}"), "/".to_string())
                    }
                } else {
                    (format!("sneakerweb.localhost:{sw_port}"), path.to_string())
                };

                let upstream_path_with_query = if let Some(q) = query {
                    format!("{}?{}", upstream_path, q)
                } else {
                    upstream_path
                };

                let method = req.method().as_str();
                let headers = req.headers();
                let body = req.body();

                let (status_code, resp_headers, resp_body) = match forward_to_upstream(
                    &upstream_host,
                    sw_port,
                    method,
                    &upstream_path_with_query,
                    headers,
                    body,
                ) {
                    Ok(res) => res,
                    Err(e) => {
                        let response = tauri::http::Response::builder()
                            .status(500)
                            .body(format!("Forward error: {e}").into_bytes())
                            .unwrap();
                        responder.respond(response);
                        return;
                    }
                };

                let mut content_type = resp_headers
                    .get("content-type")
                    .cloned()
                    .unwrap_or_default();

                let has_html_prefix = resp_body.starts_with(b"<!doctype")
                    || resp_body.starts_with(b"<!DOCTYPE")
                    || resp_body.starts_with(b"<html")
                    || resp_body.starts_with(b"<HTML");

                if content_type.is_empty() {
                    if path == "/" || path == "/oldest" || path == "/newest" || path == "/sneakiest" || has_html_prefix {
                        content_type = "text/html; charset=utf-8".to_string();
                    } else if path.ends_with(".css") {
                        content_type = "text/css".to_string();
                    } else if path.ends_with(".js") {
                        content_type = "application/javascript".to_string();
                    } else if path.ends_with(".png") {
                        content_type = "image/png".to_string();
                    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
                        content_type = "image/jpeg".to_string();
                    } else if path.ends_with(".svg") {
                        content_type = "image/svg+xml".to_string();
                    }
                }

                let is_html = content_type.contains("text/html");

                let final_body = if is_html {
                    let body_str = String::from_utf8_lossy(&resp_body);
                    let pattern = format!(
                        "http://((?:sneakerweb|[a-f0-9]{{64}})\\.localhost(?:{sw_port})?)(/[^\"'\\s>]*)?"
                    );
                    let re = regex_lite::Regex::new(&pattern);
                    match re {
                        Ok(re) => {
                            let rewritten = re.replace_all(&body_str, |caps: &regex_lite::Captures| {
                                let domain = caps.get(1).map_or("", |m| {
                                    let s = m.as_str();
                                    s.split('.').next().unwrap_or(s)
                                });
                                let path = caps.get(2).map_or("/", |m| m.as_str());
                                format!("/proxy/{domain}{path}")
                            });
                            rewritten.into_owned().into_bytes()
                        }
                        Err(_) => resp_body,
                    }
                } else {
                    resp_body
                };

                let mut response_builder = tauri::http::Response::builder().status(status_code);
                let mut content_type_added = false;
                for (k, v) in &resp_headers {
                    let kl = k.to_lowercase();
                    if kl == "content-length" || kl == "transfer-encoding" || kl == "connection" {
                        continue;
                    }
                    if kl == "content-type" {
                        content_type_added = true;
                        response_builder = response_builder.header(k, &content_type);
                    } else {
                        response_builder = response_builder.header(k, v);
                    }
                }
                if !content_type_added && !content_type.is_empty() {
                    response_builder = response_builder.header("content-type", &content_type);
                }
                response_builder = response_builder.header("Access-Control-Allow-Origin", "*");
                let response = response_builder.body(final_body).unwrap();
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

            #[cfg(target_os = "windows")]
            let home_url: url::Url = "http://sneaker.localhost/".parse().unwrap();
            #[cfg(not(target_os = "windows"))]
            let home_url: url::Url = "sneaker://localhost/".parse().unwrap();

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
                tauri::webview::WebviewBuilder::new("content", tauri::WebviewUrl::External(home_url.clone()));

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
            pick_file
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
