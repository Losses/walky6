import { spawn } from "child_process";
import { writeFileSync, existsSync, mkdirSync, readFileSync } from "fs";
import { join } from "path";
import { createServer } from "net";
import { randomUUID } from "crypto";

const PROGRESS_FILE = "/tmp/walking-viewer-import-progress";

let PORT = 3000;

const sessionId = randomUUID();
const sessionJson = JSON.stringify({ uuid: sessionId, sneakerwebPort: null, apiPort: PORT });
writeFileSync("/tmp/walking-viewer-session", sessionJson);

const SNEAKERWEB_DIR = join(import.meta.dir, ".sneakerweb_store");
if (!existsSync(SNEAKERWEB_DIR)) {
  mkdirSync(SNEAKERWEB_DIR, { recursive: true });
}

// Dynamic Port Selection for Bun server
function getFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.listen(0, () => {
      const address = server.address();
      const port = typeof address === "string" ? 0 : address?.port || 0;
      server.close(() => resolve(port));
    });
    server.on("error", (err) => reject(err));
  });
}

try {
  PORT = await getFreePort();
} catch (e) {
  console.error("Failed to find free port for Bun server, falling back to 3000");
}
let sneakerwebPort = 8080;
let sneakerwebProcess: ReturnType<typeof spawn> | null = null;

// Start sneakerweb serve — let the wrapper choose the port
async function startSneakerweb() {
  const wrapperPath = join(import.meta.dir, "target", "release", "sneakerweb-wrapper");
  if (!existsSync(wrapperPath)) {
    console.error(`ERROR: sneakerweb-wrapper not found at ${wrapperPath}. Run 'cargo build --release' first.`);
    return;
  }
  console.log(`Starting sneakerweb serve (wrapper: ${wrapperPath}, session: ${sessionId})...`);
  sneakerwebProcess = spawn(wrapperPath, ["serve"], {
    env: {
      ...process.env,
      SNEAKERWEB_DIR: SNEAKERWEB_DIR,
      SESSION_UUID: sessionId
    },
    stdio: "inherit"
  });

  sneakerwebProcess.on("error", (err: Error) => {
    console.error("Failed to spawn sneakerweb-wrapper:", err.message);
  });
  sneakerwebProcess.on("exit", (code: number | null) => {
    console.error(`sneakerweb-wrapper exited with code ${code}`);
  });

  const portFilePath = `/tmp/walking-viewer.${sessionId}.sneakerweb.port`;
  for (let i = 0; i < 50; i++) {
    await new Promise(r => setTimeout(r, 100));
    if (existsSync(portFilePath)) {
      try {
    sneakerwebPort = parseInt(readFileSync(portFilePath, "utf8").trim(), 10);
    // Update session file with actual sneakerweb port
    writeFileSync("/tmp/walking-viewer-session", JSON.stringify({ uuid: sessionId, sneakerwebPort, apiPort: PORT }));
    console.log(`Sneakerweb server port: ${sneakerwebPort}`);
        return;
      } catch (e) {}
    }
  }
  console.error("Timeout waiting for sneakerweb port file");
}

await startSneakerweb();

// Clean up server on exit
function cleanup() {
  if (sneakerwebProcess) {
    sneakerwebProcess.kill();
  }
}
process.on("exit", cleanup);
process.on("SIGTERM", () => { cleanup(); process.exit(); });
process.on("SIGINT", () => { cleanup(); process.exit(); });

Bun.serve({
  port: PORT,
  hostname: "127.0.0.1",
  async fetch(req) {
    const url = new URL(req.url);

    // API: Import .snk file
    if (url.pathname === "/api/import" && req.method === "POST") {
      try {
        const contentType = req.headers.get("content-type") || "";

        let buffer: ArrayBuffer;
        if (contentType.includes("multipart/form-data")) {
          const formData = await req.formData();
          const file = formData.get("file") as Blob | null;
          if (!file) {
            return new Response(JSON.stringify({ error: "No file uploaded" }), {
              status: 400,
              headers: { "Content-Type": "application/json" }
            });
          }
          buffer = await file.arrayBuffer();
        } else {
          buffer = await req.arrayBuffer();
        }

        const tempDir = join(import.meta.dir, "temp");
        if (!existsSync(tempDir)) {
          mkdirSync(tempDir, { recursive: true });
        }
        const tempPath = join(tempDir, `upload_${Date.now()}.snk`);
        writeFileSync(tempPath, Buffer.from(buffer));

        // Execute sneakerweb import with SNEAKERWEB_DIR environment variable
        return new Promise<Response>((resolve) => {
          const proc = spawn("./target/release/sneakerweb-wrapper", ["import", tempPath], {
            env: {
              ...process.env,
              SNEAKERWEB_DIR: SNEAKERWEB_DIR,
              IMPORT_PROGRESS_FILE: PROGRESS_FILE
            }
          });
          let stderr = "";
          proc.stderr.on("data", (data) => {
            stderr += data.toString();
          });
          proc.on("close", (code) => {
            // Clean up temp file
            try {
              Bun.file(tempPath).delete();
            } catch (e) {}

            if (code === 0) {
              resolve(new Response(JSON.stringify({ success: true }), {
                headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
              }));
            } else {
              resolve(new Response(JSON.stringify({ error: stderr.trim() || "Import failed" }), {
                status: 500,
                headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
              }));
            }
          });
        });
      } catch (err: unknown) {
        return new Response(JSON.stringify({ error: String(err) }), {
          status: 500,
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
        });
      }
    }

    // API: Import .snk file by local file path (for native client)
    if (url.pathname === "/api/import-path" && req.method === "POST") {
      try {
        const body = await req.json();
        const filePath = body.filePath;
        if (!filePath || !existsSync(filePath)) {
          return new Response(JSON.stringify({ error: "Invalid or non-existent file path" }), {
            status: 400,
            headers: { "Content-Type": "application/json" }
          });
        }

        // Execute sneakerweb-wrapper import
        return new Promise<Response>((resolve) => {
          const proc = spawn("./target/release/sneakerweb-wrapper", ["import", filePath], {
            env: {
              ...process.env,
              SNEAKERWEB_DIR: SNEAKERWEB_DIR,
              IMPORT_PROGRESS_FILE: PROGRESS_FILE
            }
          });
          let stderr = "";
          proc.stderr.on("data", (data) => {
            stderr += data.toString();
          });
          proc.on("close", (code) => {
            if (code === 0) {
              resolve(new Response(JSON.stringify({ success: true }), {
                headers: { "Content-Type": "application/json" }
              }));
            } else {
              resolve(new Response(JSON.stringify({ error: stderr.trim() || "Import failed" }), {
                status: 500,
                headers: { "Content-Type": "application/json" }
              }));
            }
          });
        });
      } catch (err: unknown) {
        return new Response(JSON.stringify({ error: String(err) }), {
          status: 500,
          headers: { "Content-Type": "application/json" }
        });
      }
    }



    // API: Import progress
    if (url.pathname === "/api/import-progress" && req.method === "GET") {
      try {
        const content = readFileSync(PROGRESS_FILE, "utf8").trim();
        return new Response(content, {
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
        });
      } catch {
        return new Response(JSON.stringify({ phase: "idle", processed: 0, total: 0, message: "" }), {
          headers: { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" }
        });
      }
    }

    // API: Get List of Sites/Domains
    if (url.pathname === "/api/sites" && req.method === "GET") {
      try {
        const response = await fetch(`http://127.0.0.1:${sneakerwebPort}/`, {
          headers: { "Host": `sneakerweb.localhost:${sneakerwebPort}` }
        });
        const html = await response.text();
        const matches = [...html.matchAll(/href="http:\/\/([a-f0-9]{64})\.localhost/g)];
        const domains = Array.from(new Set(matches.map(m => m[1]!)));
        return new Response(JSON.stringify({ domains }), {
          headers: { "Content-Type": "application/json" }
        });
      } catch (err: unknown) {
        return new Response(JSON.stringify({ error: String(err) }), {
          status: 500,
          headers: { "Content-Type": "application/json" }
        });
      }
    }

    // Proxy requests to local sneakerweb server (dynamic port)
    if (url.pathname.startsWith("/proxy/")) {
      const match = url.pathname.match(/^\/proxy\/([^\/]+)(.*)$/);
      if (match) {
        const domain = match[1]!;
        const path = match[2] || "/";
        const host = domain === "sneakerweb" ? `sneakerweb.localhost:${sneakerwebPort}` : `${domain}.localhost:${sneakerwebPort}`;

        const targetUrl = `http://127.0.0.1:${sneakerwebPort}${path}${url.search}`;
        try {
          const response = await fetch(targetUrl, {
            headers: {
              "Host": host
            }
          });

          const resHeaders = new Headers(response.headers);
          // Allow cross-origin inside our sandbox iframe
          resHeaders.set("Access-Control-Allow-Origin", "*");
          // Remove problematic compression headers so Bun can stream the response
          resHeaders.delete("content-encoding");

          const contentType = response.headers.get("content-type") || "";
          const isHtml = contentType.includes("text/html") || 
                         (domain === "sneakerweb" && (path === "/" || path === "/oldest" || path === "/newest" || path === "/sneakiest"));

          if (isHtml) {
            let htmlText = await response.text();
            
            // Rewrite any http://<domain>.localhost:<port>/path to /proxy/<domain>/path
            // Also handle sneakerweb.localhost
            htmlText = htmlText.replace(
              /http:\/\/(sneakerweb|[a-f0-9]{64})\.localhost(?::\d+)?(\/[^"' >]*)?/g,
              (match, domain, path) => {
                const cleanPath = path || "/";
                return `/proxy/${domain}${cleanPath}`;
              }
            );

            resHeaders.set("Content-Type", "text/html; charset=utf-8");
            return new Response(htmlText, {
              status: response.status,
              headers: resHeaders
            });
          }

          return new Response(response.body, {
            status: response.status,
            headers: resHeaders
          });
        } catch (err: unknown) {
          return new Response(`Proxy Error: ${String(err)}`, { status: 500 });
        }
      }
    }

    // Proxy root / directly to sneakerweb portal homepage with URL rewriting
    if (url.pathname === "/") {
      const targetUrl = `http://127.0.0.1:${sneakerwebPort}/`;
      try {
        const response = await fetch(targetUrl, {
          headers: {
            "Host": `sneakerweb.localhost:${sneakerwebPort}`
          }
        });
        const resHeaders = new Headers(response.headers);
        resHeaders.set("Access-Control-Allow-Origin", "*");
        resHeaders.delete("content-encoding");

        let htmlText = await response.text();

        // Rewrite any http://<domain>.localhost:<port>/path to /proxy/<domain>/path
        // Also handle sneakerweb.localhost
        htmlText = htmlText.replace(
          /http:\/\/(sneakerweb|[a-f0-9]{64})\.localhost(?::\d+)?(\/[^"' >]*)?/g,
          (match, domain, path) => {
            const cleanPath = path || "/";
            return `/proxy/${domain}${cleanPath}`;
          }
        );

        resHeaders.set("Content-Type", "text/html; charset=utf-8");
        return new Response(htmlText, {
          status: response.status,
          headers: resHeaders
        });
      } catch (err: unknown) {
        return new Response(`Proxy Error: ${String(err)}`, { status: 500 });
      }
    }

    return new Response("Not Found", { status: 404 });
  }
});

console.log(`Server running at http://127.0.0.1:${PORT}`);
