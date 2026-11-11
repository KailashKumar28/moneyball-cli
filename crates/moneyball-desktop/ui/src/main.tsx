import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

// Outside Tauri (vite dev, Playwright) the fixtures stand in for the shell.
// The branch is dead in a production build, so the mock never ships.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  (await import("./ipc/mock")).install();
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
