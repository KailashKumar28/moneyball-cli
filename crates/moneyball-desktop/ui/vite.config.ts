import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Tauri serves dist/ from a custom scheme, so asset paths stay relative.
export default defineConfig({
  base: "./",
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "safari15" },
});
