use clap::{Parser, Subcommand};
use smol::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use smol::net::{TcpListener, TcpStream};
use smol::stream::StreamExt;
use std::io::{BufRead, BufReader as StdBufReader};
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Parser)]
#[command(version = "1.0.0", about = "Isolated wrapper for sneakerweb")]
struct WrapperCli {
    #[arg(long, global = true)]
    dir: Option<PathBuf>,

    #[arg(long, global = true)]
    port: Option<u16>,

    #[arg(long, global = true)]
    browser: Option<PathBuf>,

    #[command(subcommand)]
    command: WrapperCommands,
}

#[derive(Subcommand)]
enum WrapperCommands {
    Domain,
    Publish { src: PathBuf, #[arg(short, long)] domain: Option<String>, #[arg(short, long)] secret: Option<String> },
    Block { domain: String },
    Import { src: PathBuf, #[arg(short, long)] mode: Option<String> },
    Export { dest: PathBuf, #[arg(short, long)] collection: Option<PathBuf> },
    Serve,
    Launch,
    FileDialog,
    PickAndImport,
}

type ProgressState = Arc<Mutex<String>>;

fn new_progress_state() -> ProgressState {
    Arc::new(Mutex::new(
        r#"{"phase":"idle","processed":0,"total":0,"message":""}"#.to_string(),
    ))
}

fn get_free_port() -> u16 {
    StdTcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(8080)
}

fn random_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let r = (ts as u64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    format!("{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (r >> 32) as u32, (r >> 16) & 0xffff, r & 0xfff,
        0x8000 | ((r >> 48) & 0x3fff), r & 0xfffffffffff)
}

fn write_port_file(port: u16) {
    if let Ok(uuid) = std::env::var("SESSION_UUID") {
        let path = format!("/tmp/walking-viewer.{uuid}.sneakerweb.port");
        let _ = std::fs::write(&path, port.to_string());
    }
}

fn ensure_sneakerweb_dir(cli_dir: Option<PathBuf>) {
    if let Some(dir) = cli_dir {
        unsafe { std::env::set_var("SNEAKERWEB_DIR", dir); }
    } else if std::env::var_os("SNEAKERWEB_DIR").is_none() {
        if let Ok(cwd) = std::env::current_dir() {
            let mut local_dir = cwd;
            local_dir.push(".sneakerweb_store");
            unsafe { std::env::set_var("SNEAKERWEB_DIR", local_dir); }
        }
    }
}

fn find_browser(cli_browser: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(b) = cli_browser {
        if b.exists() { return Some(b); }
    }
    let mut exe_path = std::env::current_exe().ok()?;
    exe_path.pop();
    exe_path.push("sneaker-reader");
    if exe_path.exists() { return Some(exe_path); }
    let cwd = PathBuf::from("./sneaker-reader");
    if cwd.exists() { return Some(cwd); }
    None
}

fn progress_json(handle: &sneakerweb::import::ProgressHandle) -> String {
    use std::sync::atomic::Ordering;
    let phase = handle.phase_name();
    let processed = handle.processed_bytes.load(Ordering::Relaxed);
    let total = handle.total_bytes.load(Ordering::Relaxed);
    let message = match phase {
        "decoding" => "Decoding entries...",
        "importing" => "Importing entries...",
        "done" => "Import complete",
        _ => "",
    };
    format!(
        r#"{{"phase":"{}","processed":{},"total":{},"message":"{}"}}"#,
        phase, processed, total, message
    )
}

fn spawn_import_with_progress(
    wrapper_path: &PathBuf,
    import_path: &str,
    sneakerweb_dir: &PathBuf,
    progress_state: &ProgressState,
) -> std::process::ExitStatus {
    let mut child = Command::new(wrapper_path)
        .arg("import")
        .arg(import_path)
        .env("SNEAKERWEB_DIR", sneakerweb_dir)
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn import");

    let stdout = child.stdout.take().unwrap();
    let state = progress_state.clone();
    std::thread::spawn(move || {
        let reader = StdBufReader::new(stdout);
        for line in reader.lines().flatten() {
            if line.starts_with("{\"phase\":") {
                if let Ok(mut s) = state.lock() {
                    *s = line;
                }
            }
        }
    });

    child.wait().expect("failed to wait on import")
}

async fn handle_import_api(
    mut stream: TcpStream,
    wrapper_path: PathBuf,
    sneakerweb_dir: PathBuf,
    progress_state: ProgressState,
) {
    let mut buf_reader = BufReader::new(stream.clone());
    let _ = buf_reader.fill_buf().await;
    let buf = buf_reader.buffer().to_vec();
    let req_str = String::from_utf8_lossy(&buf);

    let lines: Vec<&str> = req_str.lines().collect();
    let first_line = lines.first().unwrap_or(&"");
    let parts: Vec<&str> = first_line.split_whitespace().collect();

    if parts.len() >= 2 && parts[0] == "POST" && parts[1] == "/pick-file" {
        eprintln!("[pick-file] Spawning file dialog subprocess...");

        let output = Command::new(&wrapper_path)
            .arg("file-dialog")
            .env("SNEAKERWEB_DIR", &sneakerweb_dir)
            .output();

        let resp = match output {
            Ok(output) => {
                if output.status.success() {
                    let file_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    eprintln!("[pick-file] File selected: {}", file_path);

                    let path = PathBuf::from(&file_path);
                    match std::fs::read(&path) {
                        Ok(data) => {
                            let tmp = std::env::temp_dir().join(format!("sneakerweb-import-{}.snk", random_uuid()));
                            if let Err(e) = std::fs::write(&tmp, &data) {
                                format!("HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\n\r\n{{\"error\":\"failed to save temp file: {}\"}}", e)
                            } else {
                                let tmp_path = tmp.to_string_lossy().to_string();
                                let status = spawn_import_with_progress(
                                    &wrapper_path, &tmp_path, &sneakerweb_dir, &progress_state,
                                );
                                let _ = std::fs::remove_file(&tmp);

                                match status.success() {
                                    true => "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"success\":true}".to_string(),
                                    false => "HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"error\":\"import failed\"}".to_string(),
                                }
                            }
                        }
                        Err(e) => format!("HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{{\"error\":\"Failed to read file: {}\"}}", e),
                    }
                } else {
                    eprintln!("[pick-file] File dialog cancelled or failed");
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"cancelled\":true}".to_string()
                }
            }
            Err(e) => {
                eprintln!("[pick-file] Failed to spawn file dialog: {}", e);
                format!("HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{{\"error\":\"Failed to spawn file dialog: {}\"}}", e)
            }
        };
        let _ = stream.write_all(resp.as_bytes()).await;
    } else if parts.len() >= 2 && parts[0] == "POST" && parts[1] == "/api/import" {
        let content_length: usize = lines.iter()
            .find(|l| l.to_lowercase().starts_with("content-length:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);

        if content_length == 0 {
            let _ = stream.write_all(
                b"HTTP/1.1 400 BAD REQUEST\r\nContent-Type: application/json\r\n\r\n{\"error\":\"empty body\"}"
            ).await;
            return;
        }

        let body_start = req_str.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
        let mut body = buf[body_start..].to_vec();
        while body.len() < content_length {
            let _ = buf_reader.fill_buf().await;
            let b = buf_reader.buffer();
            let needed = content_length - body.len();
            body.extend_from_slice(&b[..needed.min(b.len())]);
            buf_reader.consume(needed.min(b.len()));
        }
        body.truncate(content_length);

        let tmp = std::env::temp_dir().join(format!("sneakerweb-import-{}.snk", random_uuid()));
        if std::fs::write(&tmp, &body).is_err() {
            let _ = stream.write_all(
                b"HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\n\r\n{\"error\":\"failed to save temp file\"}"
            ).await;
            return;
        }

        let tmp_path = tmp.to_string_lossy().to_string();
        let status = spawn_import_with_progress(
            &wrapper_path, &tmp_path, &sneakerweb_dir, &progress_state,
        );
        let _ = std::fs::remove_file(&tmp);

        let resp = match status.success() {
            true => "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"success\":true}",
            false => "HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"error\":\"import failed\"}",
        };
        let _ = stream.write_all(resp.as_bytes()).await;
    } else if parts.len() >= 2 && parts[0] == "POST" && parts[1] == "/api/import-path" {
        let body_start = req_str.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
        let body = &req_str[body_start..];

        let file_path = body.trim()
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .and_then(|s| s.split('"').nth(3))
            .map(|s| s.to_string())
            .unwrap_or_default();

        if file_path.is_empty() {
            let _ = stream.write_all(
                b"HTTP/1.1 400 BAD REQUEST\r\nContent-Type: application/json\r\n\r\n{\"error\":\"missing filePath\"}"
            ).await;
            return;
        }

        let status = spawn_import_with_progress(
            &wrapper_path, &file_path, &sneakerweb_dir, &progress_state,
        );

        let resp = match status.success() {
            true => "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"success\":true}",
            false => "HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"error\":\"import failed\"}",
        };
        let _ = stream.write_all(resp.as_bytes()).await;
    } else if parts.len() >= 2 && parts[0] == "GET" && parts[1] == "/api/import-progress" {
        let body = progress_state.lock()
            .map(|s| s.clone())
            .unwrap_or_else(|_| r#"{"phase":"idle","processed":0,"total":0,"message":""}"#.to_string());
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
            body
        );
        let _ = stream.write_all(resp.as_bytes()).await;
    } else if first_line.starts_with("OPTIONS") {
        let _ = stream.write_all(
            b"HTTP/1.1 204 NO CONTENT\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n"
        ).await;
    } else {
        let _ = stream.write_all(b"HTTP/1.1 404 NOT FOUND\r\n\r\n").await;
    }
}

async fn run_import_api_server(port: u16, wrapper_path: PathBuf, sneakerweb_dir: PathBuf, progress_state: ProgressState) {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await;
    let Ok(listener) = listener else { return };
    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        let wp = wrapper_path.clone();
        let sd = sneakerweb_dir.clone();
        let ps = progress_state.clone();
        smol::spawn(handle_import_api(stream, wp, sd, ps)).detach();
    }
}

fn main() -> anyhow::Result<()> {
    let cli = WrapperCli::parse();

    ensure_sneakerweb_dir(cli.dir.clone());

    let port = cli.port.or_else(|| {
        std::env::var("PORT").ok().and_then(|p| p.parse().ok())
    });
    if let Some(p) = port {
        unsafe { std::env::set_var("PORT", p.to_string()); }
    }

    match cli.command {
        WrapperCommands::Launch => {
            let uuid = random_uuid();
            let sneakerweb_dir = std::env::var("SNEAKERWEB_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(".sneakerweb_store"));
            let wrapper_path = std::env::current_exe()?;

            let api_port = get_free_port();
            let sneakerweb_port = get_free_port();

            let session_json = format!(
                r#"{{"uuid":"{uuid}","sneakerwebPort":{sneakerweb_port},"apiPort":{api_port}}}"#
            );
            std::fs::write("/tmp/walking-viewer-session", &session_json)?;

            let progress_state = new_progress_state();
            let api_dir = sneakerweb_dir.clone();
            let api_wrapper = wrapper_path.clone();
            let api_ps = progress_state.clone();
            std::thread::spawn(move || {
                smol::block_on(run_import_api_server(api_port, api_wrapper, api_dir, api_ps));
            });

            let mut serve_child = Command::new(&wrapper_path)
                .arg("serve")
                .arg("--port")
                .arg(sneakerweb_port.to_string())
                .env("SNEAKERWEB_DIR", &sneakerweb_dir)
                .spawn()?;

            std::thread::sleep(std::time::Duration::from_millis(500));

            let browser_path = find_browser(cli.browser)
                .ok_or_else(|| anyhow::anyhow!("sneaker-reader not found. Build with: npx -y @perryts/perry compile perryts_app.ts -o sneaker-reader"))?;
            eprintln!("Launching browser: {}", browser_path.display());
            let mut browser_child = Command::new(&browser_path).spawn()?;
            browser_child.wait()?;

            eprintln!("Shutting down...");
            let _ = serve_child.kill();
            let _ = serve_child.wait();
            let _ = std::fs::remove_file("/tmp/walking-viewer-session");
        }
        WrapperCommands::Domain => {
            sneakerweb::domain::generate_domain(&sneakerweb::domain::DomainArgs {});
        }
        WrapperCommands::Publish { src, domain, secret } => {
            smol::block_on(async {
                sneakerweb::publish::publish(&sneakerweb::publish::PublishArgs { src, domain, secret }).await
            })?;
        }
        WrapperCommands::Block { domain } => {
            smol::block_on(async {
                sneakerweb::block::block(&sneakerweb::block::BlockArgs { domain }).await
            })?;
        }
        WrapperCommands::Import { src, mode } => {
            let import_mode = match mode.as_deref() {
                Some("all") => Some(sneakerweb::import::ImportMode::All),
                Some("familiar") => Some(sneakerweb::import::ImportMode::Familiar),
                _ => None,
            };
            let args = sneakerweb::import::ImportArgs { src, mode: import_mode };
            let handle = sneakerweb::import::ProgressHandle::new();

            let poller_handle = handle.clone();
            let poller = std::thread::spawn(move || {
                use std::io::Write;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    let phase = poller_handle.phase.load(std::sync::atomic::Ordering::Relaxed);
                    if phase == sneakerweb::import::PHASE_DONE {
                        let json = progress_json(&poller_handle);
                        println!("{}", json);
                        let _ = std::io::stdout().flush();
                        break;
                    }
                    let json = progress_json(&poller_handle);
                    println!("{}", json);
                    let _ = std::io::stdout().flush();
                }
            });

            let result = smol::block_on(sneakerweb::import::import_sneak(&args, &handle));
            let _ = poller.join();
            result?;
        }
        WrapperCommands::Export { dest, collection } => {
            smol::block_on(async {
                sneakerweb::export::export_sneak(&sneakerweb::export::ExportArgs { dest, collection }).await
            })?;
        }
        WrapperCommands::Serve => {
            let mut port = cli.port.or_else(|| {
                std::env::var("PORT").ok().and_then(|p| p.parse().ok())
            });
            loop {
                let p = port.unwrap_or_else(get_free_port);
                eprintln!("Trying port {p}...");
                unsafe { std::env::set_var("PORT", p.to_string()); }
                write_port_file(p);
                match smol::block_on(sneakerweb::serve::start_server(p)) {
                    Ok(()) => break,
                    Err(err) => {
                        eprintln!("Port {p} failed: {err}");
                        port = None;
                    }
                }
            }
        }
        WrapperCommands::FileDialog => {
            let file = rfd::FileDialog::new()
                .add_filter("Sneaker files", &["snk"])
                .set_title("Import .snk file")
                .pick_file();
            match file {
                Some(path) => println!("{}", path.display()),
                None => std::process::exit(1),
            }
        }
        WrapperCommands::PickAndImport => {
            let file = rfd::FileDialog::new()
                .add_filter("Sneaker files", &["snk"])
                .set_title("Import .snk file")
                .pick_file();

            match file {
                Some(path) => {
                    match std::fs::read(&path) {
                        Ok(data) => {
                            let tmp = std::env::temp_dir().join(format!("sneakerweb-import-{}.snk", random_uuid()));
                            if let Err(e) = std::fs::write(&tmp, &data) {
                                eprintln!("Failed to save temp file: {}", e);
                                std::process::exit(1);
                            }

                            let tmp_path = tmp.to_string_lossy().to_string();
                            let args = sneakerweb::import::ImportArgs {
                                src: PathBuf::from(&tmp_path),
                                mode: None,
                            };
                            let handle = sneakerweb::import::ProgressHandle::new();

                            let poller_handle = handle.clone();
                            let poller = std::thread::spawn(move || {
                                use std::io::Write;
                                loop {
                                    std::thread::sleep(std::time::Duration::from_millis(200));
                                    let phase = poller_handle.phase.load(std::sync::atomic::Ordering::Relaxed);
                                    let json = progress_json(&poller_handle);
                                    println!("{}", json);
                                    let _ = std::io::stdout().flush();
                                    if phase == sneakerweb::import::PHASE_DONE {
                                        break;
                                    }
                                }
                            });

                            let result = smol::block_on(sneakerweb::import::import_sneak(&args, &handle));
                            let _ = poller.join();

                            let _ = std::fs::remove_file(&tmp);

                            match result {
                                Ok(_) => {
                                    println!("success");
                                    std::process::exit(0);
                                }
                                Err(e) => {
                                    eprintln!("Import failed: {}", e);
                                    std::process::exit(1);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to read file: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
                None => {
                    eprintln!("File selection cancelled");
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
