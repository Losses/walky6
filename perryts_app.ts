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

// Read session file (single JSON with all ports)
let sneakerwebPort = "8080";

try {
  const raw = readFileSync("/tmp/walking-viewer-session", "utf8").trim();
  const session = JSON.parse(raw);
  if (session.sneakerwebPort) sneakerwebPort = String(session.sneakerwebPort);
} catch (e) {}

// 1. Reactive states for the browser window
const homeUrl = `http://sneakerweb.localhost:${sneakerwebPort}`;
const currentUrl = State(homeUrl);
const urlInput = State(homeUrl);
const statusText = State("Done");
const isLoading = State(false);
const canGoBackState = State(false);

// 2. Initialize the main WebView widget
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

// 3. Handle import via native file picker (direct subprocess)
function handleImport() {
  console.log("[handleImport] Spawning pick-and-import subprocess...");
  statusText.set("Opening file dialog...");
  isLoading.set(true);

  const wrapperPath = "./target/release/sneakerweb-wrapper";
  const proc = spawn(wrapperPath, ["pick-and-import"], {
    env: {
      ...process.env,
      SNEAKERWEB_DIR: "./.sneakerweb_store"
    }
  });

  let stdout = "";
  let stderr = "";

  proc.stdout.on("data", (data) => {
    stdout += data.toString();
  });

  proc.stderr.on("data", (data) => {
    stderr += data.toString();
  });

  proc.on("close", (code) => {
    if (code === 0) {
      statusText.set("Successfully imported!");
      alert({ title: "Import Succeeded", message: "Drop imported successfully!" });
      webviewLoadUrl(wv, homeUrl);
    } else if (stderr.includes("cancelled")) {
      statusText.set("Done");
    } else {
      statusText.set(`Import failed: ${stderr.trim() || "unknown error"}`);
      alert({ title: "Import Failed", message: `Could not import drop:\n${stderr.trim() || "unknown error"}` });
    }
    isLoading.set(false);
  });

  proc.on("error", (err) => {
    statusText.set(`Error: ${err.message}`);
    alert({ title: "Error", message: `Failed to spawn file picker: ${err.message}` });
    isLoading.set(false);
  });
}

// 4. Lay out the application (standard GTK structure)
App({
  title: "sneakerweb",
  width: 1024,
  height: 768,
  body: VStack(0, [
    // --- Standard Buttons Toolbar ---
    HStack(8, [
      Button("<- Back", () => {
        if (canGoBackState.value) webviewGoBack(wv);
      }),
      Button("Forward ->", () => {
        webviewGoForward(wv);
      }),
      Button("Stop", () => {
        // Stop navigation
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

    // --- Address Bar ---
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

    // --- Main Browser content area (expands dynamically to fill height) ---
    wv
  ])
});
