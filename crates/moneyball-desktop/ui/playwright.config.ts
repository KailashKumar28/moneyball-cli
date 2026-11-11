import { defineConfig, devices } from "@playwright/test";

// The app runs in WKWebView, so WebKit is the only project.
export default defineConfig({
  testDir: "tests",
  fullyParallel: true,
  reporter: "list",
  use: { baseURL: "http://localhost:1420", deviceScaleFactor: 2 },
  projects: [{ name: "webkit", use: { browserName: "webkit", ...devices["Desktop Safari"], deviceScaleFactor: 2 } }],
  webServer: { command: "pnpm dev", url: "http://localhost:1420", reuseExistingServer: true },
});
