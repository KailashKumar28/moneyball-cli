// Proves the stand-in Meta with no app at all: the terminal binary, built
// with core's e2e feature, refreshes a client through it. Meta lands for
// both accounts, every capture fills, CRM is skipped (not connected), the
// report is written; then a token the stand-in does not know fails the
// refresh with Meta's code 190.
//
//   node check-mock-meta.mjs             build, then check
//   node check-mock-meta.mjs --no-build  check the binary already built
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { CLIENT, startMockMeta, TOKEN } from "./mock-meta.mjs";

const here = fileURLToPath(new URL(".", import.meta.url));
const repo = join(here, "../../..");
// Beside the app's e2e binary, never in target/debug.
const target = join(repo, "target/e2e");
const cli = join(target, "debug/moneyball");

const run = (cmd, args, opts) =>
  new Promise((resolve, reject) => {
    spawn(cmd, args, { stdio: "inherit", ...opts }).on("error", reject).on("exit", (code) => resolve(code ?? 1));
  });

if (!process.argv.includes("--no-build")) {
  const code = await run("cargo", ["build", "-p", "moneyball", "--features", "moneyball-core/e2e"], { cwd: repo, env: { ...process.env, CARGO_TARGET_DIR: target } });
  if (code !== 0) throw new Error(`the build failed (exit ${code})`);
}
if (!existsSync(cli)) throw new Error(`no binary at ${cli}: run without --no-build.`);

const meta = await startMockMeta();
const root = realpathSync(mkdtempSync(join(tmpdir(), "moneyball-meta-check-")));
const home = join(root, "home");
// Where the app's Add a Client puts it.
const ws = join(home, "Moneyball", CLIENT);
mkdirSync(join(ws, ".moneyball"), { recursive: true });
mkdirSync(join(home, ".moneyball"), { recursive: true });
const products = [
  { name: "Cityville", ad_account: "1001" },
  { name: "Fincity", ad_account: "1002" },
];
writeFileSync(join(ws, ".moneyball/config.json"), JSON.stringify({ brand: CLIENT, products }, null, 2));
const auth = join(home, ".moneyball/auth.json");
const storeToken = (token) => {
  writeFileSync(auth, JSON.stringify({ meta_access_token: token }));
  chmodSync(auth, 0o600);
};

const env = { ...process.env, HOME: home, MB_E2E_META_BASE: meta.url };
for (const key of Object.keys(env)) if (key.startsWith("MONEYBALL_") || key === "MB_AGENT") delete env[key];
const refresh = (...flags) => run(cli, ["--data-root", ws, "refresh", ...flags], { env });
const record = () => {
  const runs = join(ws, ".moneyball/runs");
  const [file] = readdirSync(runs).filter((f) => f.endsWith(".json"));
  const r = JSON.parse(readFileSync(join(runs, file), "utf8"));
  return { ...r, step: Object.fromEntries(r.steps.map((s) => [s.name, s])) };
};

let ok = false;
try {
  // Onboarding's calls, followed as core follows them (every page).
  const get = async (path) => (await fetch(`${meta.url}${path}${path.includes("?") ? "&" : "?"}access_token=${TOKEN}`)).json();
  assert.ok((await get("/me")).name);
  assert.deepEqual((await get(`/debug_token?input_token=${TOKEN}`)).data.scopes, ["ads_read"]);
  const accounts = [];
  for (let page = await get("/me/adaccounts?fields=id,name,account_status&limit=200"); ; page = await (await fetch(page.paging.next)).json()) {
    accounts.push(...page.data);
    if (!page.paging.next) break;
  }
  assert.equal(accounts.length, 3, "every page of ad accounts");
  assert.equal(accounts.filter((a) => a.name.includes(CLIENT)).length, 1, "one account to suggest");

  // 1. The refresh through the stand-in: degraded only because no CRM is connected.
  storeToken(TOKEN);
  console.log("\n== refresh with the stand-in's token");
  assert.equal(await refresh(), 2, "exit 2: degraded (no CRM)");
  const r = record();
  assert.deepEqual(r.steps.map((s) => s.name), ["meta", "assets", "crm", "report"]);
  assert.equal(r.step.meta.status, "done", JSON.stringify(r.step.meta));
  assert.equal(r.step.meta.counts.products, 2);
  assert.equal(r.step.meta.counts.rows, 3 * 28, "3 ads x 28 days");
  assert.equal(r.step.assets.status, "done", JSON.stringify(r.step.assets));
  assert.deepEqual(r.step.assets.counts, { creatives: 3, leads: 6, adsets: 2, changes: 2, ads_7d: 3 });
  assert.equal(r.step.crm.status, "skipped");
  assert.match(r.step.crm.message, /not connected|no CRM connected/);
  assert.equal(r.step.report.status, "partial");
  assert.equal(r.step.report.error_kind, "no_crm");
  assert.match(readFileSync(r.artifacts.report, "utf8"), /Cityville/);
  const images = join(ws, ".moneyball/history/assets/creatives");
  assert.equal(readdirSync(images, { recursive: true }).filter((f) => f.endsWith(".png")).length, 3, "every creative image cached");
  assert.ok(meta.log.every((l) => l.startsWith("GET ")), "nothing but GETs");

  // 2. A token the stand-in does not know: Meta's code 190, nothing lands.
  storeToken("EAAThisTokenWasRevokedLongAgo0000000000000000");
  console.log("\n== refresh with a revoked token");
  assert.equal(await refresh("--force"), 1, "exit 1: failed");
  const bad = record();
  assert.equal(bad.step.meta.status, "failed");
  assert.equal(bad.step.meta.error_kind, "meta_auth");
  assert.match(bad.step.meta.message, /code 190/);
  ok = true;
} finally {
  meta.close();
  if (ok) rmSync(root, { recursive: true, force: true });
  else console.error(`\nThe home and workspace are kept for a look: ${root}`);
}
console.log("\ncheck-mock-meta: green. The refresh lands Meta through the stand-in and writes the report.");
