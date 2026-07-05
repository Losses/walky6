use clap::{Parser, Subcommand};
use smol::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use smol::net::{TcpListener, TcpStream};
use smol::stream::StreamExt;
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use std::process::Command;

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

async fn handle_import_api(mut stream: TcpStream, wrapper_path: PathBuf, sneakerweb_dir: PathBuf) {
    let mut buf_reader = BufReader::new(stream.clone());
    let _ = buf_reader.fill_buf().await;
    let buf = buf_reader.buffer().to_vec();
    let req_str = String::from_utf8_lossy(&buf);

    let lines: Vec<&str> = req_str.lines().collect();
    let first_line = lines.first().unwrap_or(&"");
    let parts: Vec<&str> = first_line.split_whitespace().collect();

    if parts.len() >= 2 && parts[0] == "POST" && parts[1] == "/api/import-path" {
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

        let status = Command::new(&wrapper_path)
            .arg("import")
            .arg(&file_path)
            .env("SNEAKERWEB_DIR", &sneakerweb_dir)
            .status()
            .map(|s| s.success());

        let resp = match status {
            Ok(true) => "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"success\":true}",
            _ => "HTTP/1.1 500 ERROR\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\n\r\n{\"error\":\"import failed\"}",
        };
        let _ = stream.write_all(resp.as_bytes()).await;
    } else if first_line.starts_with("OPTIONS") {
        let _ = stream.write_all(
            b"HTTP/1.1 204 NO CONTENT\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n"
        ).await;
    } else {
        let _ = stream.write_all(b"HTTP/1.1 404 NOT FOUND\r\n\r\n").await;
    }
}

async fn run_import_api_server(port: u16, wrapper_path: PathBuf, sneakerweb_dir: PathBuf) {
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).await;
    let Ok(listener) = listener else { return };
    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        smol::spawn(handle_import_api(stream, wrapper_path.clone(), sneakerweb_dir.clone())).detach();
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

            // Single session file with all info
            let session_json = format!(
                r#"{{"uuid":"{uuid}","sneakerwebPort":{sneakerweb_port},"apiPort":{api_port}}}"#
            );
            std::fs::write("/tmp/walking-viewer-session", &session_json)?;

            // Start import API in background
            let api_dir = sneakerweb_dir.clone();
            let api_wrapper = wrapper_path.clone();
            std::thread::spawn(move || {
                smol::block_on(run_import_api_server(api_port, api_wrapper, api_dir));
            });

            // Start sneakerweb serve with pre-selected port
            let mut serve_child = Command::new(&wrapper_path)
                .arg("serve")
                .arg("--port")
                .arg(sneakerweb_port.to_string())
                .env("SNEAKERWEB_DIR", &sneakerweb_dir)
                .spawn()?;

            // Give serve a moment to bind
            std::thread::sleep(std::time::Duration::from_millis(500));

            // Launch browser
            let browser_path = find_browser(cli.browser)
                .ok_or_else(|| anyhow::anyhow!("sneaker-reader not found. Build with: npx -y @perryts/perry compile perryts_app.ts -o sneaker-reader"))?;
            eprintln!("Launching browser: {}", browser_path.display());
            let mut browser_child = Command::new(&browser_path).spawn()?;
            browser_child.wait()?;

            // Cleanup
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
            smol::block_on(async {
                sneakerweb::import::import_sneak(&sneakerweb::import::ImportArgs { src, mode: import_mode }).await
            })?;
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
    }

    Ok(())
}
