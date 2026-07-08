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
      try {
        await invoke("navigate_content", { path: url });
      } catch (e) {
        console.error("navigate_content failed:", e);
      }
    },
    [],
  );

  const navigateTo = useCallback(
    (urlOrPath: string) => {
      if (baseUrl === null) return;
      let url = urlOrPath;
      if (!url.startsWith("sneaker://")) {
        if (urlOrPath === "/" || urlOrPath === "") {
          url = "sneaker://home/";
        } else {
          const cleanPath = urlOrPath.startsWith("/") ? urlOrPath.slice(1) : urlOrPath;
          url = `${baseUrl}${cleanPath}`;
        }
      }
      navigateToInternal(url);
    },
    [baseUrl, navigateToInternal],
  );

  useEffect(() => {
    invoke<string>("get_base_url").then((url) => {
      setBaseUrl(url);
    }).catch(err => {
      console.error("Failed to get base URL:", err);
    });
  }, []);

  useEffect(() => {
    if (baseUrl !== null) {
      navigateTo("sneaker://home/");
    }
  }, [baseUrl, navigateTo]);

  useEffect(() => {
    interface NavPayload {
      url: string;
      action: 'push' | 'replace' | 'pop' | 'load';
    }

    const unlisten = listen<NavPayload>("webview-navigated", (event) => {
      const { url: newUrl, action } = event.payload;

      // Handle progress page
      if (newUrl.includes("/__progress__")) {
        setUrlInput("Importing...");
        return;
      }

      // Check where we are in history
      const currentUrl = historyStack.current[historyIndex.current];
      if (currentUrl === newUrl) {
        setUrlInput(newUrl);
        updateNavButtons();
        return;
      }

      // Check if it's a replace navigation
      if (action === 'replace') {
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
        const index = historyStack.current.indexOf(newUrl);
        if (index !== -1) {
          historyIndex.current = index;
          setUrlInput(newUrl);
          updateNavButtons();
          return;
        }
      }

      // Otherwise, new navigation (push or load)
      historyStack.current = historyStack.current.slice(0, historyIndex.current + 1);
      historyStack.current.push(newUrl);
      historyIndex.current = historyStack.current.length - 1;
      setUrlInput(newUrl);
      updateNavButtons();
    });

    return () => {
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

    navigateToInternal(destination);
  }, [urlInput, baseUrl, navigateToInternal]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === "Go") {
        e.preventDefault();
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
