import {
  App,
  VStack,
  HStack,
  Text,
  Button,
  TextField,
  WebView,
  State,
  webviewLoadUrl,
  webviewReload,
  webviewGoBack,
  webviewGoForward,
  webviewCanGoBack,
  alert,
} from "perry/ui";
import { readFileSync } from "fs";
import { spawn } from "child_process";

let sneakerwebPort = "8080";

try {
  const raw = readFileSync("/tmp/walking-viewer-session", "utf8").trim();
  const session = JSON.parse(raw);
  if (session.sneakerwebPort) sneakerwebPort = String(session.sneakerwebPort);
} catch (e) {}

const homeUrl = `http://sneakerweb.localhost:${sneakerwebPort}`;
const currentUrl = State(homeUrl);
const urlInput = State(homeUrl);
const statusText = State("Done");
const isLoading = State(false);
const canGoBackState = State(false);
const progressText = State("");

function formatProgress(progress: any): string {
  const { phase, processed, total, message } = progress;
  if (phase === "idle" || !total || total === 0) return "";
  if (phase === "done") return "Import complete!";

  let pct: number;
  if (phase === "importing") {
    pct = 95;
  } else {
    pct = Math.min(90, Math.floor((processed / total) * 100));
  }

  const filled = Math.floor(pct / 100 * 30);
  const empty = 30 - filled;
  const bar = "\u2588".repeat(filled) + "\u2591".repeat(empty);
  return `[${bar}] ${pct}% ${message || phase}`;
}

const wv = WebView({
  url: currentUrl.value,
  width: -1,
  height: -1,
  onShouldNavigate: (url: string) => {
    if (url.startsWith("sneakerweb://")) {
      return false;
    }
    urlInput.set(url);
    statusText.set(`Opening page ${url}...`);
    isLoading.set(true);
    return true;
  },
  onLoaded: () => {
    isLoading.set(false);
    statusText.set("Done");
    currentUrl.set(urlInput.value);
    canGoBackState.set(webviewCanGoBack(wv) === 1);
  },
  onError: (err: string) => {
    isLoading.set(false);
    statusText.set(`Error loading page: ${err}`);
  }
});

function handleImport() {
  console.log("[handleImport] Spawning pick-and-import subprocess...");
  statusText.set("Opening file dialog...");
  isLoading.set(true);
  progressText.set("Waiting for file selection...");

  const wrapperPath = "./target/release/sneakerweb-wrapper";
  const proc = spawn(wrapperPath, ["pick-and-import"], {
    env: {
      ...process.env,
      SNEAKERWEB_DIR: "./.sneakerweb_store"
    }
  });

  let stderr = "";
  let stdoutBuf = "";

  proc.stdout.on("data", (data) => {
    stdoutBuf += data.toString();
    const lines = stdoutBuf.split("\n");
    stdoutBuf = lines.pop() || "";
    for (const line of lines) {
      if (line.startsWith('{"phase":')) {
        try {
          const progress = JSON.parse(line);
          const text = formatProgress(progress);
          if (text) progressText.set(text);
        } catch {}
      }
    }
  });

  proc.stderr.on("data", (data) => {
    stderr += data.toString();
  });

  proc.on("close", (code) => {
    if (code === 0) {
      statusText.set("Successfully imported!");
      progressText.set("Import complete!");
      alert({ title: "Import Succeeded", message: "Drop imported successfully!" });
      webviewLoadUrl(wv, homeUrl);
    } else if (stderr.includes("cancelled")) {
      statusText.set("Done");
      progressText.set("");
    } else {
      statusText.set(`Import failed: ${stderr.trim() || "unknown error"}`);
      progressText.set("Import failed");
      alert({ title: "Import Failed", message: `Could not import drop:\n${stderr.trim() || "unknown error"}` });
    }
    isLoading.set(false);
  });

  proc.on("error", (err) => {
    statusText.set(`Error: ${err.message}`);
    progressText.set("Import error");
    alert({ title: "Error", message: `Failed to spawn file picker: ${err.message}` });
    isLoading.set(false);
  });
}

App({
  title: "sneakerweb",
  width: 1024,
  height: 768,
  body: VStack(0, [
    HStack(8, [
      Button("<- Back", () => {
        if (canGoBackState.value) webviewGoBack(wv);
      }),
      Button("Forward ->", () => {
        webviewGoForward(wv);
      }),
      Button("Stop", () => {
      }),
      Button("Refresh", () => {
        webviewReload(wv);
      }),
      Button("Home", () => {
        webviewLoadUrl(wv, homeUrl);
      }),
      Button("Import .snk", () => {
        handleImport();
      })
    ]),

    HStack(6, [
      Text("Address:"),
      TextField(urlInput.value, (newVal: string) => {
        urlInput.set(newVal);
      }),
      Button("Go", () => {
        let destination = urlInput.value.trim();
        if (!destination.startsWith("http://") && !destination.startsWith("https://")) {
          destination = `http://${destination}`;
        }
        webviewLoadUrl(wv, destination);
      })
    ]),

    Text(progressText.value),

    wv
  ])
});
