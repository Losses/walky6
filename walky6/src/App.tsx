import { useState, useCallback, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

interface ImportProgress {
  phase: string;
  processed: number;
  total: number;
  message: string;
}

function App() {
  const [proxyPort, setProxyPort] = useState<number | null>(null);
  const [urlInput, setUrlInput] = useState("");
  const [iframeUrl, setIframeUrl] = useState("");
  const [isImporting, setIsImporting] = useState(false);
  const [progressLabel, setProgressLabel] = useState("");
  const [progressPercent, setProgressPercent] = useState(0);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const historyStack = useRef<string[]>([]);
  const historyIndex = useRef(-1);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  const homePath = "/";

  useEffect(() => {
    invoke<number>("get_proxy_port").then((port) => {
      setProxyPort(port);
      const homeUrl = `http://127.0.0.1:${port}/`;
      setUrlInput(homeUrl);
    });

    const unlisten = listen<ImportProgress>("import-progress", (event) => {
      const { phase, processed, total, message } = event.payload;
      if (phase === "decoding" && total > 0) {
        const pct = Math.min(90, Math.floor((processed / total) * 100));
        setProgressLabel(`${message} (${pct}%)`);
        setProgressPercent(pct);
      } else if (phase === "importing") {
        setProgressLabel(message);
        setProgressPercent(90);
      } else if (phase === "done") {
        setProgressLabel("Import complete!");
        setProgressPercent(100);
      }
    });

    const handleMessage = (event: MessageEvent) => {
      if (event.data && event.data.type === 'walky6_navigation') {
        const path = event.data.path;
        if (proxyPort !== null) {
          setUrlInput(`http://127.0.0.1:${proxyPort}${path}`);
        }
      }
    };
    window.addEventListener('message', handleMessage);

    return () => {
      unlisten.then((fn) => fn());
      window.removeEventListener('message', handleMessage);
    };
  }, [proxyPort]);

  useEffect(() => {
    if (proxyPort !== null) {
      navigateToInternal(homePath);
    }
  }, [proxyPort]);

  const navigateToInternal = useCallback(async (path: string) => {
    if (proxyPort === null) return;
    const fullUrl = `http://127.0.0.1:${proxyPort}${path}`;
    setUrlInput(fullUrl);
    setIframeUrl(fullUrl);

    if (historyIndex.current < historyStack.current.length - 1) {
      historyStack.current = historyStack.current.slice(0, historyIndex.current + 1);
    }
    historyStack.current.push(path);
    historyIndex.current = historyStack.current.length - 1;
    updateNavButtons();
  }, [proxyPort]);

  const navigateTo = useCallback((path: string) => {
    navigateToInternal(path);
  }, [navigateToInternal]);

  const updateNavButtons = useCallback(() => {
    setCanGoBack(historyIndex.current > 0);
    setCanGoForward(historyIndex.current < historyStack.current.length - 1);
  }, []);

  const goBack = useCallback(() => {
    if (historyIndex.current > 0 && proxyPort !== null) {
      historyIndex.current--;
      const path = historyStack.current[historyIndex.current];
      setUrlInput(`http://127.0.0.1:${proxyPort}${path}`);
      iframeRef.current?.contentWindow?.postMessage({ type: 'walky6_go_back' }, '*');
      updateNavButtons();
    }
  }, [proxyPort, updateNavButtons]);

  const goForward = useCallback(() => {
    if (historyIndex.current < historyStack.current.length - 1 && proxyPort !== null) {
      historyIndex.current++;
      const path = historyStack.current[historyIndex.current];
      setUrlInput(`http://127.0.0.1:${proxyPort}${path}`);
      iframeRef.current?.contentWindow?.postMessage({ type: 'walky6_go_forward' }, '*');
      updateNavButtons();
    }
  }, [proxyPort, updateNavButtons]);

  const goHome = useCallback(() => {
    navigateTo(homePath);
  }, [navigateTo, homePath]);

  const refresh = useCallback(() => {
    if (proxyPort !== null) {
      iframeRef.current?.contentWindow?.postMessage({ type: 'walky6_refresh' }, '*');
    }
  }, [proxyPort]);

  const handleGo = useCallback(() => {
    const destination = urlInput.trim();
    if (proxyPort === null) return;

    const baseUrl = `http://127.0.0.1:${proxyPort}`;
    if (destination.startsWith(baseUrl)) {
      const path = destination.slice(baseUrl.length) || "/";
      navigateTo(path);
    } else if (!destination.startsWith("http://") && !destination.startsWith("https://")) {
      const path = destination.startsWith("/") ? destination : `/${destination}`;
      navigateTo(path);
    }
  }, [urlInput, proxyPort, navigateTo]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      handleGo();
    }
  }, [handleGo]);

  const handleImport = useCallback(async () => {
    setIsImporting(true);
    setProgressLabel("Opening file dialog...");
    setProgressPercent(0);

    try {
      await invoke("pick_and_import");
      setProgressLabel("Import complete!");
      setProgressPercent(100);
      setTimeout(() => {
        setIsImporting(false);
        setProgressLabel("");
        setProgressPercent(0);
        refresh();
      }, 1500);
    } catch (err) {
      const msg = String(err);
      if (msg.includes("cancelled")) {
        setIsImporting(false);
        setProgressLabel("");
      } else {
        setProgressLabel(`Import failed: ${msg}`);
        setTimeout(() => {
          setIsImporting(false);
          setProgressLabel("");
          setProgressPercent(0);
        }, 3000);
      }
    }
  }, [refresh]);

  return (
    <div className="app">
      <div className="toolbar">
        <button onClick={goBack} disabled={!canGoBack} title="Back">
          {"<"} Back
        </button>
        <button onClick={goForward} disabled={!canGoForward} title="Forward">
          Forward {">"}
        </button>
        <button onClick={refresh} title="Refresh">
          Refresh
        </button>
        <button onClick={goHome} title="Home">
          Home
        </button>
        <button onClick={handleImport} disabled={isImporting} title="Import .snk file">
          Import .snk
        </button>
      </div>
      <div className="address-bar">
        <span className="address-label">Address:</span>
        <input
          type="text"
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          onKeyDown={handleKeyDown}
          className="address-input"
        />
        <button onClick={handleGo} className="go-btn">
          Go
        </button>
      </div>
      {isImporting && (
        <div className="import-overlay">
          <div className="import-box">
            <div className="progress-bar-container">
              <div
                className="progress-bar-fill"
                style={{ width: `${progressPercent}%` }}
              />
            </div>
            <p className="progress-label">{progressLabel}</p>
          </div>
        </div>
      )}
      <div className="content-container">
        {iframeUrl && (
          <iframe
            ref={iframeRef}
            src={iframeUrl}
            className="content-iframe"
          />
        )}
      </div>
    </div>
  );
}

export default App;
