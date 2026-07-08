import "98.css";
import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import loadingSvg from "./assets/data-loading.svg";
import "./Progress.css";

function ProgressView() {
  const [percent, setPercent] = useState(0);
  const [message, setMessage] = useState("Preparing import...");

  useEffect(() => {
    let pollCount = 0;
    const poll = async () => {
      pollCount++;
      try {
        const res = await fetch("/__progress_api__");
        const data = await res.json();

        console.log(
          `[progress] poll #${pollCount}: is_importing=${data.is_importing}, ` +
          `phase=${data.phase}, processed_bytes=${data.processed_bytes}, ` +
          `total_bytes=${data.total_bytes}, processed_entries=${data.processed_entries}`
        );

        if (data.total_bytes > 0) {
          const pct = Math.min(
            100,
            Math.floor((data.processed_bytes / data.total_bytes) * 100),
          );
          console.log(`[progress] setting percent=${pct} (${data.processed_bytes}/${data.total_bytes})`);
          setPercent(pct);
        } else {
          console.log(`[progress] total_bytes is 0, not updating percent`);
        }

        if (data.phase === "decoding") {
          setMessage(`Decoding entries... (${percent}%)`);
        } else if (data.phase === "importing") {
          setMessage(`Importing entries... (${percent}%)`);
        } else if (data.phase === "done") {
          setMessage("Import complete!");
          setPercent(100);
        } else if (data.phase === "error") {
          setMessage(`Import failed: ${data.message || "unknown error"}`);
        }
      } catch (err) {
        console.error("[progress] Failed to poll progress:", err);
      }
    };

    const interval = setInterval(poll, 200);
    poll();

    return () => {
      console.log("[progress] cleaning up interval");
      clearInterval(interval);
    };
  }, [percent]);

  return (
    <div className="progress-wrapper">
      <div className="window progress-window">
        <div className="progress-content">
          <div className="progress-row">
            <div
              className="loading-icon"
              style={{ backgroundImage: `url(${loadingSvg})` }}
            ></div>
            <div className="progress-bar-wrap">
              <div className="progress-indicator">
                <span
                  className="progress-indicator-bar"
                  style={{ width: `${percent}%` }}
                ></span>
              </div>
            </div>
          </div>
          <p className="progress-text">{message}</p>
        </div>
      </div>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ProgressView />
  </React.StrictMode>,
);
