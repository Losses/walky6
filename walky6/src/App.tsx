import { useState, useCallback, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

interface ImportProgress {
  phase: string;
  processed: number;
  total: number;
  processed_entries: number;
  message: string;
}

function App() {
  const [proxyPort, setProxyPort] = useState<number | null>(null);
  const [urlInput, setUrlInput] = useState("");
  const [isImporting, setIsImporting] = useState(false);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const historyStack = useRef<string[]>([]);
  const historyIndex = useRef(-1);

  const homePath = "/";

  useEffect(() => {
    invoke<number>("get_proxy_port").then((port) => {
      setProxyPort(port);
      const homeUrl = `http://127.0.0.1:${port}/`;
      setUrlInput(homeUrl);
    });

    const unlisten = listen<ImportProgress>("import-progress", (event) => {
      // Progress is now displayed in the content webview's progress page
      // We still listen to detect when import is done
      const { phase } = event.payload;
      if (phase === "done") {
        setTimeout(() => {
          setIsImporting(false);
          navigateTo(homePath);
        }, 1500);
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (proxyPort !== null) {
      navigateToInternal(homePath);
    }
  }, [proxyPort]);

  const navigateToInternal = useCallback(
    async (path: string) => {
      if (proxyPort === null) return;
      const fullUrl = `http://127.0.0.1:${proxyPort}${path}`;
      setUrlInput(fullUrl);

      if (historyIndex.current < historyStack.current.length - 1) {
        historyStack.current = historyStack.current.slice(
          0,
          historyIndex.current + 1,
        );
      }
      historyStack.current.push(path);
      historyIndex.current = historyStack.current.length - 1;
      updateNavButtons();

      try {
        await invoke("navigate_content", { path });
      } catch (e) {
        console.error("navigate_content failed:", e);
      }
    },
    [proxyPort],
  );

  const navigateTo = useCallback(
    (path: string) => {
      navigateToInternal(path);
    },
    [navigateToInternal],
  );

  const updateNavButtons = useCallback(() => {
    setCanGoBack(historyIndex.current > 0);
    setCanGoForward(historyIndex.current < historyStack.current.length - 1);
  }, []);

  const goBack = useCallback(() => {
    if (historyIndex.current > 0 && proxyPort !== null) {
      historyIndex.current--;
      const path = historyStack.current[historyIndex.current];
      setUrlInput(`http://127.0.0.1:${proxyPort}${path}`);
      invoke("navigate_content", { path }).catch(console.error);
      updateNavButtons();
    }
  }, [proxyPort, updateNavButtons]);

  const goForward = useCallback(() => {
    if (
      historyIndex.current < historyStack.current.length - 1 &&
      proxyPort !== null
    ) {
      historyIndex.current++;
      const path = historyStack.current[historyIndex.current];
      setUrlInput(`http://127.0.0.1:${proxyPort}${path}`);
      invoke("navigate_content", { path }).catch(console.error);
      updateNavButtons();
    }
  }, [proxyPort, updateNavButtons]);

  const goHome = useCallback(() => {
    navigateTo(homePath);
  }, [navigateTo, homePath]);

  const refresh = useCallback(() => {
    if (proxyPort !== null) {
      const path = historyStack.current[historyIndex.current] || homePath;
      invoke("navigate_content", { path }).catch(console.error);
    }
  }, [proxyPort, homePath]);

  const handleGo = useCallback(() => {
    const destination = urlInput.trim();
    if (proxyPort === null) return;

    const baseUrl = `http://127.0.0.1:${proxyPort}`;
    if (destination.startsWith(baseUrl)) {
      const path = destination.slice(baseUrl.length) || "/";
      navigateTo(path);
    } else if (
      !destination.startsWith("http://") &&
      !destination.startsWith("https://")
    ) {
      const path = destination.startsWith("/")
        ? destination
        : `/${destination}`;
      navigateTo(path);
    }
  }, [urlInput, proxyPort, navigateTo]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        handleGo();
      }
    },
    [handleGo],
  );

  const handleImport = useCallback(async () => {
    setIsImporting(true);

    try {
      await invoke("navigate_content", { path: "/__progress__" });
      await invoke("pick_and_import");
      // Import completed successfully - navigation handled by event listener
    } catch (err) {
      const msg = String(err);
      if (msg.includes("cancelled")) {
        setIsImporting(false);
        navigateTo(homePath);
      } else {
        // Import failed - show error briefly then return home
        console.error("Import failed:", msg);
        setTimeout(() => {
          setIsImporting(false);
          navigateTo(homePath);
        }, 3000);
      }
    }
  }, [navigateTo, homePath]);

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
