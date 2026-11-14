// The one command: build the app with the e2e feature, give it a home and
// a workspace of its own, start the stand-in model and the stand-in Meta,
// and drive the real binary three times: twice on the demo workspace (the
// second launch proves a session survives a restart), then once on a new,
// empty home, where it onboards a client.
//
//   node run.mjs                         build, then run every launch
//   node run.mjs --no-build              run the binary already built
//   node run.mjs --no-build onboarding   only the launches named
import { spawn } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { startMockMeta } from "./mock-meta.mjs";
import { startMockModel } from "./mock-model.mjs";

const here = fileURLToPath(new URL(".", import.meta.url));
const crate = join(here, "..");
// Its own target folder: the e2e binary can never be mistaken for, or
// overwrite, a normal build.
const target = join(crate, "../../target/e2e");
const app = join(target, "debug/moneyball-desktop");

/** Run a command to its end, output passed through; resolves to its exit code. */
const run = (cmd, args, opts) =>
  new Promise((resolve, reject) => {
    spawn(cmd, args, { stdio: "inherit", ...opts }).on("error", reject).on("exit", (code) => resolve(code ?? 1));
  });

async function build() {
  const cli = join(crate, "ui/node_modules/.bin/tauri");
  if (!existsSync(cli)) throw new Error("ui/node_modules is missing: run `pnpm install` in crates/moneyball-desktop/ui first.");
  const args = ["build", "--debug", "--no-bundle", "--features", "e2e", "--config", "e2e/tauri.e2e.conf.json"];
  const code = await run(cli, args, { cwd: crate, env: { ...process.env, CARGO_TARGET_DIR: target } });
  if (code !== 0) throw new Error(`the e2e build failed (exit ${code})`);
}

/** A temp home and a temp copy of the demo workspace, wired to the mock model. */
function hermetic(mockUrl) {
  // Resolved, as a real home is: macOS's temp dir sits behind a symlink.
  const root = realpathSync(mkdtempSync(join(tmpdir(), "moneyball-e2e-")));
  const home = join(root, "home");
  // Inside the temp home, so the app shows it as a person's would: "~/acme-homes".
  const workspace = join(home, "acme-homes");
  const dot = join(workspace, ".moneyball");
  cpSync(join(crate, "fixtures/snap"), join(dot, "history/snap"), { recursive: true });
  // The app reads "last refresh" from the snapshot's file time: date each
  // copy as the morning pull that would have written it.
  for (const date of readdirSync(join(dot, "history/snap"))) {
    const pulled = new Date(`${date}T06:00:00`);
    for (const file of readdirSync(join(dot, "history/snap", date))) utimesSync(join(dot, "history/snap", date, file), pulled, pulled);
    utimesSync(join(dot, "history/snap", date), pulled, pulled);
  }
  const config = JSON.parse(readFileSync(join(crate, "fixtures/config.json"), "utf8"));
  config.model_provider = "mock";
  config.model = "mock-1";
  config.model_providers = { mock: { name: "Mock", base_url: `${mockUrl}/v1`, wire_api: "chat_completions" } };
  writeFileSync(join(dot, "config.json"), JSON.stringify(config, null, 2));

  // A session the terminal app wrote in this workspace, slash command and
  // all: the Mac app must neither list it nor hand it to the model.
  mkdirSync(join(dot, "sessions"), { recursive: true });
  const header = { session: { id: "mb-20261008T050000Z-tui1", started_at: "2026-10-08T05:00:00Z", data_root: workspace, front_end: "tui" } };
  const typed = { type: "user", text: "/funnel Namma Mane" };
  writeFileSync(join(dot, "sessions/mb-20261008T050000Z-tui1.jsonl"), `${JSON.stringify(header)}\n${JSON.stringify(typed)}\n`);

  mkdirSync(join(home, ".moneyball"), { recursive: true });
  writeFileSync(join(home, ".moneyball/config.json"), JSON.stringify({ last_workspace: workspace }));
  const auth = join(home, ".moneyball/auth.json");
  // Fake on purpose, and no Meta token: a refresh must fail honestly.
  writeFileSync(auth, JSON.stringify({ llm_keys: { mock: "fake-e2e-key" } }));
  chmodSync(auth, 0o600);

  // A Mac that has never run Moneyball: nothing remembered, no token.
  const fresh = join(root, "fresh");
  mkdirSync(fresh);
  return { root, home, workspace, fresh };
}

/** The harness's environment: nothing of the person's Moneyball setup leaks in. */
function cleanEnv(extra) {
  const env = { ...process.env, ...extra };
  for (const key of Object.keys(env)) {
    if (key.startsWith("MONEYBALL_") || key === "MB_AGENT") delete env[key];
  }
  return env;
}

async function main() {
  if (!process.argv.includes("--no-build")) await build();
  if (!existsSync(app)) throw new Error(`no e2e binary at ${app}: run without --no-build.`);

  const mock = await startMockModel();
  // Each daily pull held 3 s: long enough to watch the first refresh run.
  const meta = await startMockMeta({ slowMs: 3000 });
  const dirs = hermetic(mock.url);
  rmSync(join(here, "shots"), { recursive: true, force: true });
  mkdirSync(join(here, "shots"));
  const env = cleanEnv({ MB_E2E_APP: app, MB_E2E_WORKSPACE: dirs.workspace, MB_E2E_MOCK: mock.url, MB_E2E_META: meta.url });

  // One launch per spec file, in this order, each on its home.
  const launches = [
    ["smoke", dirs.home],
    ["relaunch", dirs.home],
    ["onboarding", dirs.fresh],
  ];
  const named = process.argv.slice(2).filter((a) => !a.startsWith("--"));
  let code = 0;
  try {
    for (const [spec, home] of launches) {
      if (named.length > 0 && !named.includes(spec)) continue;
      code = await run(join(here, "node_modules/.bin/wdio"), ["run", "wdio.conf.mjs", "--spec", `specs/${spec}.mjs`], { cwd: here, env: { ...env, MB_E2E_HOME: home } });
      if (code !== 0) break;
    }
  } finally {
    mock.close();
    meta.close();
    if (code === 0) rmSync(dirs.root, { recursive: true, force: true });
    else console.error(`\nFailed. The home and workspace are kept for a look: ${dirs.root}`);
  }
  console.log(code === 0 ? "\ne2e: green. Screenshots are in e2e/shots/." : "\ne2e: FAILED.");
  process.exit(code);
}

main().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
