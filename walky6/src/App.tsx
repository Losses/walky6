import { useState, useCallback, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

function App() {
  const [baseUrl, setBaseUrl] = useState<string | null>(null);
  const [urlInput, setUrlInput] = useState("");
  const [isImporting, setIsImporting] = useState(false);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const historyStack = useRef<string[]>([]);
  const historyIndex = useRef(-1);

  const updateNavButtons = useCallback(() => {
    setCanGoBack(historyIndex.current > 0);
    setCanGoForward(historyIndex.current < historyStack.current.length - 1);
  }, []);

  const navigateToInternal = useCallback(
    async (url: string) => {
      console.log("[Toolbar App.tsx] navigateToInternal called with URL:", url);
      try {
        await invoke("navigate_content", { path: url });
        console.log("[Toolbar App.tsx] navigate_content completed for URL:", url);
      } catch (e) {
        console.error("[Toolbar App.tsx] navigate_content failed for URL:", url, "error:", e);
      }
    },
    [],
  );

  const navigateTo = useCallback(
    (urlOrPath: string) => {
      console.log("[Toolbar App.tsx] navigateTo called with:", urlOrPath);
      if (baseUrl === null) {
        console.warn("[Toolbar App.tsx] baseUrl is null, skipping navigation");
        return;
      }
      let url = urlOrPath;
      if (!url.startsWith("sneaker://")) {
        if (urlOrPath === "/" || urlOrPath === "") {
          url = "sneaker://home/";
        } else {
          const cleanPath = urlOrPath.startsWith("/") ? urlOrPath.slice(1) : urlOrPath;
          url = `${baseUrl}${cleanPath}`;
        }
      }
      console.log("[Toolbar App.tsx] target URL after mapping:", url);
      navigateToInternal(url);
    },
    [baseUrl, navigateToInternal],
  );

  useEffect(() => {
    console.log("[Toolbar App.tsx] Requesting base URL from Rust...");
    invoke<string>("get_base_url").then((url) => {
      console.log("[Toolbar App.tsx] Base URL loaded:", url);
      setBaseUrl(url);
    }).catch(err => {
      console.error("[Toolbar App.tsx] Failed to get base URL:", err);
    });
  }, []);

  useEffect(() => {
    if (baseUrl !== null) {
      console.log("[Toolbar App.tsx] baseUrl is set. Initializing navigation to sneaker://home/...");
      navigateTo("sneaker://home/");
    }
  }, [baseUrl, navigateTo]);

  useEffect(() => {
    interface NavPayload {
      url: string;
      action: 'push' | 'replace' | 'pop' | 'load';
    }

    console.log("[Toolbar App.tsx] Setting up webview-navigated listener...");
    const unlisten = listen<NavPayload>("webview-navigated", (event) => {
      console.log("[Toolbar App.tsx] Received webview-navigated event:", event);
      const { url: newUrl, action } = event.payload;
      console.log("[Toolbar App.tsx] Payload details - url:", newUrl, "action:", action);

      // Handle progress page
      if (newUrl.includes("/__progress__")) {
        console.log("[Toolbar App.tsx] Navigating to progress page, setting 'Importing...'");
        setUrlInput("Importing...");
        return;
      }

      // Check where we are in history
      const currentUrl = historyStack.current[historyIndex.current];
      console.log("[Toolbar App.tsx] History index:", historyIndex.current, "Current URL in history:", currentUrl);
      if (currentUrl === newUrl) {
        console.log("[Toolbar App.tsx] URL matches current history page. Updating UI only.");
        setUrlInput(newUrl);
        updateNavButtons();
        return;
      }

      // Check if it's a replace navigation
      if (action === 'replace') {
        console.log("[Toolbar App.tsx] Action is replace. Updating stack.");
        if (historyIndex.current >= 0) {
          historyStack.current[historyIndex.current] = newUrl;
        } else {
          historyStack.current = [newUrl];
          historyIndex.current = 0;
        }
        setUrlInput(newUrl);
        updateNavButtons();
        return;
      }

      // Check if it's a popState navigation (back/forward history traversal)
      if (action === 'pop') {
        console.log("[Toolbar App.tsx] Action is pop.");
        const index = historyStack.current.indexOf(newUrl);
        if (index !== -1) {
          console.log("[Toolbar App.tsx] Found URL in stack at index:", index);
          historyIndex.current = index;
          setUrlInput(newUrl);
          updateNavButtons();
          return;
        } else {
          console.warn("[Toolbar App.tsx] Pop action URL not found in history stack:", newUrl);
        }
      }

      // Otherwise, new navigation (push or load)
      console.log("[Toolbar App.tsx] Adding new URL to history stack:", newUrl);
      historyStack.current = historyStack.current.slice(0, historyIndex.current + 1);
      historyStack.current.push(newUrl);
      historyIndex.current = historyStack.current.length - 1;
      setUrlInput(newUrl);
      updateNavButtons();
    });

    return () => {
      console.log("[Toolbar App.tsx] Cleaning up webview-navigated listener");
      unlisten.then((f) => f());
    };
  }, [updateNavButtons]);

  const goBack = useCallback(() => {
    if (historyIndex.current > 0) {
      invoke("go_back_content").catch(console.error);
    }
  }, []);

  const goForward = useCallback(() => {
    if (historyIndex.current < historyStack.current.length - 1) {
      invoke("go_forward_content").catch(console.error);
    }
  }, []);

  const goHome = useCallback(() => {
    navigateTo("sneaker://home/");
  }, [navigateTo]);

  const refresh = useCallback(() => {
    if (historyIndex.current >= 0) {
      const currentUrl = historyStack.current[historyIndex.current];
      invoke("navigate_content", { path: currentUrl }).catch(console.error);
    } else {
      navigateTo("sneaker://home/");
    }
  }, [navigateTo]);

  const handleGo = useCallback(() => {
    let destination = urlInput.trim();
    if (baseUrl === null) return;

    if (!destination.startsWith("sneaker://")) {
      const hashRegex = /^[a-fA-F0-9]{64}/;
      if (hashRegex.test(destination)) {
        destination = `sneaker://${destination}`;
      } else {
        const cleanPath = destination.startsWith("/") ? destination.slice(1) : destination;
        destination = `${baseUrl}${cleanPath}`;
      }
    }

    navigateTo(destination);
  }, [urlInput, baseUrl, navigateTo]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        handleGo();
      }
    },
    [handleGo],
  );

  const handleImport = async () => {
    try {
      const filePath = await invoke<string>("pick_file");
      
      setIsImporting(true);
      
      // Store current URL before navigating to progress page
      const previousUrl = urlInput;
      
      // Navigate to progress page
      await invoke("navigate_content", { path: "/__progress__" });
      setUrlInput("Importing...");
      
      // Start import
      await invoke("import_file", { filePath });
      
      // Import completed, navigate back to previous URL
      setTimeout(() => {
        invoke("navigate_content", { path: previousUrl });
        setIsImporting(false);
      }, 1500);
    } catch (err) {
      console.error("Import failed:", err);
      setIsImporting(false);
    }
  };

  return (
    <div className="window app-window">
      <div className="toolbar">
        <button
          onClick={goBack}
          disabled={!canGoBack || isImporting}
          className="toolbar-btn"
          title="Back"
        >
          <img src="/icons/back.svg" alt="" />
          <span>Back</span>
        </button>
        <button
          onClick={goForward}
          disabled={!canGoForward || isImporting}
          className="toolbar-btn"
          title="Forward"
        >
          <img src="/icons/forward.svg" alt="" />
          <span>Forward</span>
        </button>
        <button
          onClick={refresh}
          disabled={isImporting}
          className="toolbar-btn"
          title="Refresh"
        >
          <img src="/icons/refresh.svg" alt="" />
          <span>Refresh</span>
        </button>
        <button
          onClick={goHome}
          disabled={isImporting}
          className="toolbar-btn"
          title="Home"
        >
          <img src="/icons/home.svg" alt="" />
          <span>Home</span>
        </button>
        <button
          onClick={handleImport}
          disabled={isImporting}
          className="toolbar-btn"
          title="Import .snk file"
        >
          <img src="/icons/import.svg" alt="" />
          <span>Import</span>
        </button>
      </div>
      <div className="field-row address-bar">
        <label htmlFor="addr-input">Address</label>
        <input
          id="addr-input"
          type="text"
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={isImporting}
        />
        <button onClick={handleGo} disabled={isImporting} className="go-btn">
          <span className="label">Go</span>
        </button>
      </div>
    </div>
  );
}

export default App;
