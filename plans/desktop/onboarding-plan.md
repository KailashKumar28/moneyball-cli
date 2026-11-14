# Onboarding a new client - plan

Proposed 2026-11-14. Nothing here is built yet. Inputs, read before
changing a decision:

- The review walk of 2026-11-14: a brand-new client from install, and a
  real client's daily screens, on the real app (captures and screenshots
  were in the session; the findings are summarised below).
- An independent UI/UX expert review of that evidence.
- Research into how comparable tools onboard and what onboarding evidence
  says (sources inline).

## Decisions to confirm

The plan uses the defaults below. Each is the owner's call; changing one
changes the steps that name it.

| # | Decision | Default in this plan | Alternative |
|---|---|---|---|
| D1 | Who owns onboarding | The Mac app. The terminal wizard stays, built on the same core steps. | Keep setup terminal-only. |
| D2 | How Meta connects | A guided system-user token: paste once per Mac, checked at once, never expires. | Facebook login (OAuth): needs a server, Meta App Review and Business Verification. |
| D3 | Where a client lives | `~/Moneyball/<Client name>`, made for you; "Change..." for another folder; existing folders still open with Cmd-O. | Always ask for a folder. |
| D4 | What is required | Meta and the client's ad accounts. Everything else (goals, CRM, morning refresh, advisor key) is a checklist, asked when first needed. | A longer linear wizard. |
| D5 | CRM in the app | The preset CRMs (LeadZump, LeadSquared) as a form with a test pull. "Another CRM" (drafted by the model from a pasted request) stays in the terminal for now. | All CRMs in the app. |
| D6 | Scheduling from the app | The app ships the `moneyball` CLI inside its bundle and schedules that. | Scheduling stays terminal-only. |
| D7 | Goals | Asked per product, unit spelled out ("qualified leads a day"), blank allowed; a blank goal says "no goal set" where the goal is used. | Keep the silent default of 10 for every product. |

## What is wrong today (2026-11-14 walk)

- The Mac app cannot onboard anyone: its first screen says "Setup runs in
  the terminal for now".
- In the terminal, a client without a Meta token is stuck at step 2
  ("'skip' is no longer supported"), and the only help is
  "developers.facebook.com -> Tools -> Marketing API", which is not where
  a lasting token comes from.
- Leaving setup loses it: `/setup` starts again at step 1.
- Products are shown as raw ad-account ids; goals are free text
  ("Name=10") with no unit; the model step asks about API dialects.
- The CRM and the morning refresh are not in the wizard at all; they are
  later commands a new user has to know about.
- After setup nothing shows until a refresh is started by hand, and the
  app then repeats "No data" on every screen.
- The 2026-11-14 quick fixes (commit 27a2b61) removed the dead ends between
  `/connect`, `/connect crm` and `/setup`, and the raw errors; the flow
  itself is unchanged.

## What the research says (and what we take from it)

- Every comparable tool connects Meta with "log in with Facebook" and an
  ad-account picker; none asks for a pasted token (Triple Whale,
  AgencyAnalytics, Madgicx, Birch, Supermetrics docs). Moneyball has no
  server, so it cannot do that without Meta App Review and Business
  Verification
  ([Meta](https://developers.facebook.com/docs/facebook-login/facebook-login-for-business)).
  So: a token, but guided to the right kind (a system user's, set to never
  expire,
  [Meta](https://developers.facebook.com/docs/business-management-apis/system-users/install-apps-and-generate-tokens)),
  checked on paste, stored once for all clients.
- An account picker that shows each account's state (connected, available,
  no permission) and why (Madgicx).
- One Meta connection serves every client, and one reconnect fixes all of
  them (AgencyAnalytics, Supermetrics shared connections).
- Defer what is not essential and ask for access when the feature is
  first used ([Apple HIG, onboarding](https://developer.apple.com/design/human-interface-guidelines/onboarding)).
- A short wizard says how many steps there are and can be left and resumed
  ([NN/g, wizards](https://www.nngroup.com/articles/wizards/)).
- Never show an empty "no data" screen while data is coming; show progress,
  and link each empty area to the step that fills it
  ([NN/g, empty states](https://www.nngroup.com/articles/empty-state-interface-design/)).
- An error says how to fix itself, in place
  ([NN/g, error messages](https://www.nngroup.com/articles/error-message-guidelines/)).

## Second research round (2026-11-14): reference code and best-in-class products

Source: openai/codex @ b1573f1 and badlogic/pi-mono @ 3035f08, read for
how they build onboarding; product docs of Shopify, Superhuman, Stripe,
Fivetran, Plaid, Vercel, Raycast, Linear, Slack, Tailscale, Zapier, and
Apple's HIG. What it changes, folded into the flow below:

- **Steps are derived, never stored** (codex `onboarding_screen.rs`: each
  step is `Hidden | InProgress | Complete`, recomputed from what is on disk
  at every launch; Stripe hosted onboarding re-checks what is still
  missing). Resume is therefore free: there is no progress file.
- **Errors stay inside the step** (codex `auth.rs`): the error sits under
  the field, the typed input is kept, it clears on the next edit, and a
  failed save leaves the step open with nothing written.
- **One function, thin callers** (codex `login_with_api_key` from both CLI
  and TUI; pi's `AuthInteraction` for prompts across three front-ends).
  Our `connections::Prompter` already has pi's shape.
- **Config writes re-read under a lock** (codex `ConfigEditsBuilder`, pi
  `settings-manager`). Ours writes the whole in-memory config, so the app
  and the TUI on one client can undo each other's change.
- **Optional checklists get ignored** (Superhuman: zero impact on
  activation; full-screen setup steps with smart defaults took completion
  from 30% to over 98%). So the CRM, the one step that unlocks qualified
  leads and verdicts, gets its own prominent panel after the first
  refresh, not an equal checklist line.
- **Newest data first** (Fivetran priority-first sync, Plaid's first 30
  days in seconds): pull yesterday, show it, then fill the 28 days.
- **The Mac way** (HIG Settings, Sheets, Launching): account-wide
  connections live in a Settings window (Cmd-,) with an Accounts pane;
  setup that is part of one task is a sheet, one sheet at a time; a launch
  restores the last client and view.
- **Repair, don't recreate** (Plaid update mode, Zapier): a reconnect keeps
  every client's mapping, then pulls the days that were missed and says
  which; notices clear themselves once repaired; warn before an expiring
  token runs out.
- **Check a token for what it is** (Meta `debug_token`: `is_valid`,
  `expires_at`, `scopes`): show "never expires" and `ads_read`; warn, not
  block, on an expiring token or write scopes. Catch an obvious wrong paste
  (an app id or app secret) before calling Meta.

## The flow

### Step 0 - First launch, no client yet

The window shows one sentence on what Moneyball does and the read-only
promise, then the primary button **Add a client** and a quieter **Open a
folder...** (Cmd-O) for a workspace made in the terminal. No terminal
command is shown here any more.

### Step 1 - Connect Meta (once per Mac)

Skipped entirely when a working token is already stored.

1. A short guide in three numbered lines, with a button that opens Meta
   Business Settings > Users > System users: create a system user, give it
   the ad accounts, generate a token with `ads_read` and expiry "Never".
2. Paste the token. It is checked at once (`meta::validate_token`), and
   the error appears under the field in words: "Meta says this token
   expired or was revoked (code 190). Generate a new one in Business
   Settings."
3. On success: "Connected as <name>. It can see 26 ad accounts. Never
   expires, read-only (ads_read)." from `debug_token`; an expiring token or
   one with write scopes is a warning, not a stop. The token is stored with
   `secrets::store_meta_token` (already global to the Mac,
   `~/.moneyball/auth.json`).

The guide sits beside the masked field, not on a screen before it. When
Meta is not connected yet this is the first page of the Add-a-client sheet
(with Back), never a second sheet. Later it lives in Settings (Cmd-,) >
Accounts, the one place for the Mac-wide connection.

### Step 2 - Add the client (name, then its accounts)

1. **Client name** ("Fincity"). The folder is shown under it,
   `~/Moneyball/Fincity`, with Change... (D3). A name already used by
   another client is refused in place.
2. **Pick its ad accounts** from `meta::list_ad_accounts`, one row each:
   - the account name and id, its status from Meta (active, disabled,
     unsettled) and, when another client already uses it, "used by
     <client>";
   - a checkbox, and an editable product name, prefilled from the account
     name;
   - a search box once there are more than 10 accounts;
   - accounts whose name contains the client's name arrive ticked.
3. **Create** writes the workspace config (products, a minted
   workspace id) and opens the client. The header says "Step 2 of 2"
   throughout; leaving keeps nothing half-written.

### Step 3 - The first refresh starts on its own

Today opens with the refresh already running, its four steps live (the
existing Refresh view), and one line: "Pulling the last 28 days from Meta.
About 2 minutes. You can leave this window." With no CRM yet the CRM step
says "not connected yet" instead of failing.

The pull goes newest first: yesterday lands and shows within seconds, then
the rest of the 28 days fill in, with progress in words ("Meta: 9 of 28
days"). Today shows yesterday's report (Meta numbers only) as soon as it is
in, with a note that CRM, qualified and verdicts arrive once the CRM is
connected. That is the first value: spend, Meta leads and cost per lead
per ad set, seconds after the client was created.

Right under it sits the one prominent next step, **Connect your CRM**,
because it is what turns leads into qualified leads and verdicts (an equal
checklist line would be ignored). It can be skipped.

A failed first refresh says why and what to do in the same place (an
expired token links back to Step 1; a disabled account names it).

### Step 4 - The setup checklist on Today

A compact checklist ("3 of 5 done") sits above the report until every
item is done or dismissed with its X. Items tick themselves from what is
on disk. "Meta connected" and "Accounts chosen" arrive ticked. Each open
item is one line and one button:

- **Goals** - "Qualified leads a day you aim for, per product." A small
  form, one number per product, blank allowed (D7).
- **Connect the CRM** - pick LeadZump or LeadSquared, fill its fields
  (from `crm::presets`), **Test**: a live 28-day pull that answers in
  words: "Found 214 leads from your ads in the last 28 days." Unknown CRM
  stages are mapped in the same panel. A 401 says "key rejected, paste a
  new one"; a 400 names what the CRM refused. "Another CRM" shows the
  terminal command (D5).
- **Morning refresh** - offered once the first refresh has succeeded: one
  switch, on at 07:30 by default, with the time editable (D6). It says
  first that macOS will show "Background Items Added", and the checklist
  notices when the item is turned off in Login Items.
- **Advisor** - not on the checklist. The first time the Advisor is opened
  without a model it asks for one there: provider by plain name, model,
  key, **Test**.

Every empty area that waits on an item links to it (the CRM columns, the
verdicts, the advisor).

### Step 5 - The next client

Add a client again from the client switcher at the top of the sidebar
(its menu ends with "Add a client...", and it has a shortcut, as Linear's
and Slack's do). Meta is already connected, so it starts at Step 2;
accounts already used by a client say so. The switcher lists every client
with its status (fresh, stale, failing).

### Step 6 - When a connection breaks later

- An expired Meta token puts one notice on every client: "Meta stopped
  accepting the token on <day>. Reconnect." Reconnect is Step 1 again, and
  it keeps every client's accounts and products; then it pulls the days
  that were missed and says which, and the notices clear themselves. A
  token with an expiry date is warned about before it runs out.
- A CRM that fails says since when and the fix for that cause (the
  2026-11-14 fixes already do this in words; the button opens the CRM
  panel of Step 4 instead of a terminal command).
- A **Connections** panel (from the sidebar) shows Meta, CRM, advisor and
  schedule for the open client, each with its state and its fix, the same
  rows as `connections::status_all`.

## Shape (engineering)

Binding patterns: ARCHITECTURE 5b (direct commands, Channel events, state
in Rust, the pinned IPC contract, core words every sentence) and 6 (a new
provider is a preset).

### Core first (wave 1)

The terminal wizard holds its own save logic today
(`moneyball-tui/src/setup/steps.rs`). Move each step into core as a plain
function the two front-ends call, with the TUI switched over and its
output unchanged:

- `onboarding::steps(..)`: each step `Hidden | InProgress | Complete`,
  derived from disk (is a token stored, does the client's config exist with
  products), as codex derives its onboarding. Both the app's first screen
  and the TUI's resume read it.
- `config::update(root, |cfg| ..)`: lock, re-read, change, write privately
  (the shape of `secrets::update`). Every onboarding write and the client
  list go through it; `AppConfig::save` of a stale copy does not.
- `meta::graph_base()` replaces the seven uses of `META_GRAPH_BASE`; its
  override is read only when core is built with a new `e2e` feature
  (forwarded from the app's), and a unit test proves a normal build ignores
  it. Never a config key: the token would go wherever the config pointed.
- `~/.moneyball` is created 0700 (pi does), beside auth.json's 0600.

- `onboarding::create_client(root, name, products) -> AppConfig`: the
  folder, the config, the minted workspace id (today's `advance_save`).
- `onboarding::accounts(token, known_clients) -> Vec<AccountChoice>`:
  `list_ad_accounts` plus status words and "used by <client>".
- `onboarding::checklist(cfg) -> Vec<Item>`: what is done and what is
  left, from config, secrets, `schedule::installed` and the run records.
- CRM: split `connections::crm::preset` into steps that need no
  `Prompter`: the preset's fields; store keys and save the spec; the test
  pull as a typed result (tickets, pages, dropped, unknown stages); save a
  stage map. The terminal's `Prompter` flow becomes a loop over the same
  steps.
- Goals: `goal_for` returns `Option`; the brief, funnel and prompt say "no
  goal set" instead of using 10 (D7).
- A client list: the app remembers the clients it has opened or created
  (its own key in `~/.moneyball/config.json`, beside `last_workspace`, so
  CLI and TUI resolution never change).

### Mac app (waves 2 to 4)

New commands (each a thin wrapper over the core function, IPC.md first):
`meta_status`, `meta_connect { token }`, `client_accounts`,
`client_create { name, folder?, products }`, `clients_list`,
`client_open { path }`, `checklist`, `goals_set`, `crm_presets`,
`crm_connect { preset, fields }` (test pull progress over a Channel),
`crm_map_stages`, `schedule_set { on, at }`, `model_set { provider, model,
key }` (with a live test). No command returns a secret.

Screens: the first-launch screen, the Meta guide, Add a client, the
checklist above Today's report, the CRM panel, the Advisor's first-use
model form, the client switcher, and the Connections panel.

Scheduling (D6): bundle the CLI as a Tauri sidecar and pass its path to
`schedule::plan`, which today takes the running executable (the CLI).

### Terminal

The wizard keeps working on the same core steps, with the 2026-11-14
wording fixes, and gains resume (it reopens at the first step not done).
It stays the place for "another CRM".

### Tests

- Core: a unit test per step (create_client, accounts with each status,
  checklist states, the CRM steps on a recorded sample, goals unset).
- Mac app: fixtures pinned for every new shape; a Playwright state per
  screen (empty, error, success) at 1200x800 and 900x600, light and dark.
- End to end: the real app onboarding a client from an empty home against
  a stand-in Meta (a local server answering `/me` and `/me/adaccounts`,
  like the stand-in model). `meta::META_GRAPH_BASE` is a constant today;
  it needs an override compiled only under the `e2e` feature, so the
  release build cannot be pointed elsewhere.

## Waves

| Wave | Delivers | Done when |
|---|---|---|
| 1 | `onboarding::steps`, `create_client`, `accounts`, `checklist`; `config::update`; `meta::graph_base` behind core's `e2e` feature; CRM preset steps without a `Prompter`; goals optional; client list; TUI on all of it, unchanged | Workspace gates green; TUI render examples diffed against master |
| 2 | First launch, Connect Meta, Add a client, first refresh starts on its own | e2e onboards a client from an empty home against the stand-in Meta |
| 3 | The checklist: goals, CRM presets with test pull and stage map, morning refresh, advisor first use | e2e connects a stand-in CRM; Playwright states for every panel |
| 4 | Client switcher, next client from Step 2, Connections panel, one reconnect for all clients | e2e with two clients sharing one Meta token |
| 5 | The terminal wizard's resume and wording; docs (README, IPC.md, design notes) | A new user can follow the README from install to a report |

Every wave ends with the definition of done in AGENTS.md: build, test,
clippy, `pnpm check`, the real-app e2e and its screenshots read.

## Not in this plan

- Facebook login (D2's alternative). Revisit if Moneyball ever runs a
  server.
- "Another CRM" in the app (D5).
- The screen restructure and a Share action for the report, from the same
  review; they get their own plan.
- A demo workspace to explore before connecting anything. Worth doing if
  first-time users stall at Step 1; not needed for an operator onboarding
  their own clients.

## Open questions

1. Does a client's ad account shared into the operator's Business Manager
   work with a system-user token under standard access? Meta's docs say
   standard access covers "your own ad account"; test with one real client
   account before wave 2.
2. Should goals stay "qualified leads a day", or be set per month (how
   clients usually state targets)?
3. Does any client use a CRM other than the two presets? If most do, D5
   should flip and "another CRM" moves into the app in wave 3.
