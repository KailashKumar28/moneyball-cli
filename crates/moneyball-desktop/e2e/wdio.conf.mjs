// WebdriverIO against the real app. run.mjs sets the MB_E2E_* variables
// and starts this once per launch; the service spawns the binary and
// talks to the WebDriver server the e2e feature compiles into it.
const { MB_E2E_APP, MB_E2E_HOME, MB_E2E_META } = process.env;
if (!MB_E2E_APP || !MB_E2E_HOME || !MB_E2E_META) throw new Error("Start the tests with `node run.mjs`: it prepares the app's home and workspace.");

export const config = {
  runner: "local",
  specs: ["./specs/*.mjs"],
  maxInstances: 1,
  capabilities: [{ browserName: "tauri", "tauri:options": { application: MB_E2E_APP } }],
  services: [
    [
      "@wdio/tauri-service",
      {
        driverProvider: "embedded",
        // HOME is what moneyball-core reads; CFFIXED_USER_HOME moves the
        // webview's own storage (~/Library) into the temp home as well.
        // MB_E2E_META_BASE points every Graph call at the stand-in Meta
        // (read only by the e2e build: meta::graph_base).
        env: { HOME: MB_E2E_HOME, CFFIXED_USER_HOME: MB_E2E_HOME, MB_E2E_META_BASE: MB_E2E_META },
        captureBackendLogs: true,
        captureFrontendLogs: true,
      },
    ],
  ],
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 60_000, bail: true },
  reporters: ["spec"],
  logLevel: "warn",
  outputDir: "./logs",
  waitforTimeout: 10_000,
  // A failed step leaves a picture of what the app showed instead.
  afterTest: async (test, _context, { passed }) => {
    if (!passed) await browser.saveScreenshot(`./shots/FAILED-${test.title.replace(/[^A-Za-z0-9]+/g, "-")}.png`).catch(() => {});
  },
};
