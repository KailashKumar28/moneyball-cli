# Moneyball for Mac: end to end tests

Drives the real app binary (Tauri 2, the system WKWebView, real IPC, real
`moneyball-core`) with WebdriverIO, asserts on what is on screen, and
saves a screenshot per step. The Playwright tests in `ui/` cover the
interface against a mocked shell; this covers the two halves together.

## Run

```sh
pnpm install --dir ../ui      # once: the Tauri CLI and the interface's deps
pnpm install                  # once, in this folder
node run.mjs                  # build, then run (about 20 s after the first build)
node run.mjs --no-build       # run the binary already built
node run.mjs --no-build onboarding   # only the launches named (relaunch needs smoke)
node check-mock-meta.mjs      # the stand-in Meta on its own, against the terminal binary
```

Needs a logged-in macOS desktop session (the app opens its window; it
takes focus for a few seconds). No permission prompt: screenshots come
from WebKit itself, not from Screen Recording. Green ends with
`e2e: green`, exit code 0. Screenshots land in `shots/` (ignored by git),
WebdriverIO's and the app's logs in `logs/`. A failed step also saves
`shots/FAILED-<step>.png` and keeps the temp home for a look.

## How it works

| Piece | What it does |
|---|---|
| `run.mjs` | The one command. Builds, makes the homes and workspace, starts the mock model and the stand-in Meta, then runs WebdriverIO once per launch. |
| `tauri.e2e.conf.json` | Merged over `../tauri.conf.json` for this build only: `withGlobalTauri`, the `e2e` capability (`wdio:default`, `wdio-webdriver:default`), its own identifier and its own `frontendDist`. |
| `vite.config.mjs` | Builds `ui/` into `e2e/dist` with `@wdio/tauri-plugin` imported ahead of the entry file. |
| `mock-model.mjs` | OpenAI Chat Completions over SSE on loopback: a `brief` tool call, then streamed text; a slow stream for Stop. |
| `mock-meta.mjs` | The Graph API on loopback: `/me`, `/debug_token`, `/me/adaccounts` (two pages), and every call a refresh makes (insights, `?ids=` ads and ad sets, adimages, leads, activities, the images). Any token but its `TOKEN` gets Meta's code 190. The app reaches it through `MB_E2E_META_BASE`, which only the `e2e` build reads (`meta::graph_base`). |
| `check-mock-meta.mjs` | Proves the stand-in with no app: `moneyball refresh` (built with `moneyball-core/e2e`) lands Meta through it and writes the report; a revoked token fails with code 190. |
| `wdio.conf.mjs` | `@wdio/tauri-service`, embedded provider: the service starts the app and talks to the WebDriver server inside it. |
| `specs/smoke.mjs`, `specs/relaunch.mjs` | First launch, then a second launch on the same home. |
| `specs/onboarding.mjs` | A third launch on a new, empty home: onboarding a client against the stand-in Meta. |

Test-only code stays out of a normal build:

- The Rust plugins (`tauri-plugin-wdio`, `tauri-plugin-wdio-webdriver`)
  are optional dependencies behind the `e2e` cargo feature, registered
  under `#[cfg(feature = "e2e")]`. Check: `cargo tree -p moneyball-desktop
  | grep -c wdio` prints 0.
- `../tauri.conf.json` and `../capabilities/` do not mention them. The
  permissions and `withGlobalTauri` exist only in `tauri.e2e.conf.json`,
  which only `run.mjs` passes (`tauri build --config`). The overlay
  without the feature does not compile (unknown permission); the feature
  without the overlay starts a server the page cannot answer.
- `ui/` holds no test code at all. The page helper is added by
  `vite.config.mjs` and written to `e2e/dist`, never to `ui/dist`.
- The binary goes to `target/e2e/`, never to `target/debug` or
  `target/release`, and has its own identifier (`ai.moneyball.desktop.e2e`),
  so it does not share the single-instance lock or webview storage with
  an installed Moneyball.

Hermetic:

- The app runs with `HOME` and `CFFIXED_USER_HOME` set to a new temp
  folder. `~/.moneyball/config.json` there remembers `~/acme-homes`, a
  copy of `../fixtures` (the demo workspace) with each snapshot's file
  time set to its own morning. The real `~/.moneyball` and `~/Library`
  are not read or written. `MONEYBALL_*` and `MB_AGENT` are removed from
  the environment.
- `auth.json` holds one fake model key and no Meta token.
- The workspace's model provider is the mock on `127.0.0.1`. Model calls
  are made by Rust, so the app's CSP is as shipped.
- Meta is the stand-in on `127.0.0.1` for every launch; the first two have
  no token, so they never reach it. The onboarding launch gets a second
  home, empty: no `last_workspace`, no `auth.json`.

## What it covers

1. Launch lands on Today with the remembered workspace and its health.
2. Cmd-2 shows the Brief table with both products.
3. Funnel: product, days and level each change the table.
4. Report: none yet, Generate, the report shows in the sandboxed frame
   and its inline styles are applied under the shipped CSP (checked on
   the screenshot's pixels: the frame's document is out of reach).
5. Refresh from the footer: four steps, Meta fails with no token, the
   rest are skipped, and the notice gives the Terminal fix in words (no
   slash command).
6. Advisor: a message, the `brief` tool row with the real tool output,
   the streamed answer as markdown, exactly two model calls; Stop on a
   slow stream; a message starting with `/` is refused with no model
   call; New Session.
7. Title bar: on every screen the top-left 80x40 (where the traffic
   lights sit) holds no control and is all drag region.
8. After a relaunch the session is in the list and reopens (a session
   the terminal app wrote in the same workspace is never listed); the
   failed refresh shows in full from its run record; a product added to
   the workspace's config shows when the window regains focus.
9. Onboarding from an empty home: "Add your first client", Connect Meta
   (a revoked token is refused with code 190, the pasted one shows who it
   is and "Never expires"), the client named and the one account whose
   name holds it arrives ticked, Create Client, the first refresh live
   (Meta landed, CRM not connected), then Today shows the day's report.
   The client's config holds that account; the stand-in saw GETs only.

## What it cannot do

- See window chrome. The screenshot is the webview's content, so the
  traffic lights are not in it; their place is checked by geometry only.
- Send real key or mouse events. The driver dispatches DOM events inside
  the page: the app's own shortcuts (Cmd-1 to 5, Cmd-N, Enter, Esc) work,
  but native menu shortcuts, a real window drag and the native folder
  picker (Cmd-O) are not exercised.
- Open a `<select>`'s native popup: `choose()` sets the value and fires
  `change`.
- Read inside the report's frame (`sandbox=""`), by design.
- Show hover, or focus rings as a mouse user gets them: an element the
  driver clicks is focused programmatically, so screenshots show a
  focus ring a mouse click would not.
- Pick the theme: the app follows the system appearance.
- Test the signed release bundle or `tauri dev`. A successful refresh
  runs against the stand-in Meta, not graph.facebook.com. The binary is a debug build of the same code and the
  shipped CSP (`tauri build --debug`).

## Versions

`@wdio/tauri-service`, `@wdio/tauri-plugin`, `tauri-plugin-wdio` and
`tauri-plugin-wdio-webdriver` 1.5.1; WebdriverIO 9.31 (the service does
not support 10 yet). Verified on macOS 26.6 (arm64).
