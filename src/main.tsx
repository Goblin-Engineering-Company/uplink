import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import "./styles/themes.css";
import "./styles/app.css";

// Which surface is this? The tray dropdown runs the same bundle in a window
// labeled "tray"; everything else is the full window.
let label = "main";
try {
  label = getCurrentWindow().label;
} catch {
  // running in a plain browser (vite preview) — treat as full window
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App surface={label === "tray" ? "tray" : "window"} />
  </React.StrictMode>
);
