import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/app.css";
import App from "./App";
import { initTheme, initSystemAccent } from "./lib/theme";

// Apply the persisted theme synchronously before the first paint (no FOUC),
// then ask the host for the live macOS accent color.
initTheme();
initSystemAccent();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
