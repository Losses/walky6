import { useState, useCallback, useRef, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [baseUrl, setBaseUrl] = useState<string | null>(null);
  const [urlInput, setUrlInput] = useState("");
  const [isImporting, setIsImporting] = useState(false);
  const [canGoBack, setCanGoBack] = useState(false);
  const [canGoForward, setCanGoForward] = useState(false);
  const historyStack = useRef<string[]>([]);
  const historyIndex = useRef(-1);

  const homePath = "/";

  useEffect(() => {
    invoke<string>("get_base_url").then((url) => {
      setBaseUrl(url);
      setUrlInput(url);
    });
  }, []);

  useEffect(() => {
    if (baseUrl !== null) {
      navigateToInternal(homePath);
    }
  }, [baseUrl]);

  const navigateToInternal = useCallback(
    async (path: string) => {
      if (baseUrl === null) return;
      const cleanPath = path.startsWith("/") ? path.slice(1) : path;
      const fullUrl = `${baseUrl}${cleanPath}`;
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
    [baseUrl],
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
    if (historyIndex.current > 0 && baseUrl !== null) {
      historyIndex.current--;
      const path = historyStack.current[historyIndex.current];
      const cleanPath = path.startsWith("/") ? path.slice(1) : path;
      setUrlInput(`${baseUrl}${cleanPath}`);
      invoke("navigate_content", { path }).catch(console.error);
      updateNavButtons();
    }
  }, [baseUrl, updateNavButtons]);

  const goForward = useCallback(() => {
    if (
      historyIndex.current < historyStack.current.length - 1 &&
      baseUrl !== null
    ) {
      historyIndex.current++;
      const path = historyStack.current[historyIndex.current];
      const cleanPath = path.startsWith("/") ? path.slice(1) : path;
      setUrlInput(`${baseUrl}${cleanPath}`);
      invoke("navigate_content", { path }).catch(console.error);
      updateNavButtons();
    }
  }, [baseUrl, updateNavButtons]);

  const goHome = useCallback(() => {
    navigateTo(homePath);
  }, [navigateTo, homePath]);

  const refresh = useCallback(() => {
    if (baseUrl !== null) {
      const path = historyStack.current[historyIndex.current] || homePath;
      invoke("navigate_content", { path }).catch(console.error);
    }
  }, [baseUrl, homePath]);

  const handleGo = useCallback(() => {
    const destination = urlInput.trim();
    if (baseUrl === null) return;

    if (destination.startsWith(baseUrl)) {
      const path = "/" + destination.slice(baseUrl.length);
      navigateTo(path);
    } else if (
      !destination.startsWith("http://") &&
      !destination.startsWith("https://") &&
      !destination.startsWith("sneaker://")
    ) {
      const path = destination.startsWith("/")
        ? destination
        : `/${destination}`;
      navigateTo(path);
    }
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
        const url = new URL(previousUrl);
        const path = url.pathname + url.search + url.hash;
        invoke("navigate_content", { path });
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
