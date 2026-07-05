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
  openFileDialog
} from "perry/ui";
import { readFileSync } from "fs";

// Read session file (single JSON with all ports)
let sneakerwebPort = "8080";
let apiPort = "3000";

try {
  const raw = readFileSync("/tmp/walking-viewer-session", "utf8").trim();
  const session = JSON.parse(raw);
  if (session.sneakerwebPort) sneakerwebPort = String(session.sneakerwebPort);
  if (session.apiPort) apiPort = String(session.apiPort);
} catch (e) {}

// 1. Reactive states for the browser window
const homeUrl = `http://sneakerweb.localhost:${sneakerwebPort}`;
const currentUrl = State(homeUrl);
const urlInput = State(homeUrl);
const statusText = State("Done");
const isLoading = State(false);
const canGoBackState = State(false);

// 2. Initialize the WebView widget
const wv = WebView({
  url: currentUrl.value,
  width: -1,
  height: -1,
  onShouldNavigate: (url: string) => {
    urlInput.set(url);
    statusText.set(`Opening page ${url}...`);
    isLoading.set(true);
    return true; // Allow navigation
  },
  onLoaded: () => {
    isLoading.set(false);
    statusText.set("Done");
    // Update navigation states
    currentUrl.set(urlInput.value);
    canGoBackState.set(webviewCanGoBack(wv) === 1);
  },
  onError: (err: string) => {
    isLoading.set(false);
    statusText.set(`Error loading page: ${err}`);
  }
});

// 3. File import handler using native openFileDialog (without filters to prevent native TypeErrors)
async function handleImport() {
  statusText.set("Opening file dialog...");
  try {
    const rawPath = await openFileDialog({
      title: "Open .snk drop file"
    });

    if (!rawPath) {
      statusText.set("Done");
      return;
    }

    // Clean URI file:// scheme if present
    let filePath = rawPath.trim();
    if (filePath.startsWith("file://")) {
      filePath = filePath.slice(7);
    }

    isLoading.set(true);
    statusText.set(`Importing ${filePath}...`);

    const response = await fetch(`http://127.0.0.1:${apiPort}/api/import-path`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json"
      },
      body: JSON.stringify({ filePath })
    });

    isLoading.set(false);
    if (response.ok) {
      statusText.set("Successfully imported!");
      alert({ title: "Import Succeeded", message: "Drop imported successfully!" });
      webviewReload(wv);
    } else {
      const errData = (await response.json()) as any;
      const errMsg = errData.error || "Import failed";
      statusText.set(`Import failed: ${errMsg}`);
      alert({
        title: "Import Failed",
        message: `Could not import drop:\n${errMsg}`
      });
    }
  } catch (error: any) {
    isLoading.set(false);
    statusText.set("Error importing file.");
    alert({ title: "Error", message: error.message });
  }
}

// 4. Lay out the application (standard GTK structure)
App({
  title: "sneakerweb - Microsoft Internet Explorer",
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
