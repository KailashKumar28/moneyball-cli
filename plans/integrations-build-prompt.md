# Build prompt: connect any client's ads and CRM

You are building **connect** for moneyball-cli (`/Users/Kailash/DEV/moneyball-cli`). A new client sets up a folder and an AI model, nothing else. Then, from inside moneyball, they connect their ad platforms and their CRM: in the browser where the platform allows it, like Claude Code's login, and through a guided key where it does not.

The target design is the flow board's sections **L** (connect any client, L1 to L14), **M** (connect without a browser here, M1 to M7) and **X** (when a connection goes wrong, X1 to X11) in `plans/prototype/board.html`. Each frame's **Copy for agent** gives its exact cells, every styled run with its ratatui `Style`, and the build notes; the Build plan tab orders the tasks.

Read the repo's `CLAUDE.md`, `AGENTS.md` and `ARCHITECTURE.md` first. They are binding: the network boundary, E2E first, `cargo install` is shipping, ASCII-only authored TUI strings, never advertise a command before it works.

The design came from a four-engineer review panel on 9 Oct 2026, covering identity and security, Meta, Google Ads and CRMs, and UX and architecture. The facts below are from their reading of each platform's documentation. Two items need a spike before anyone commits to them; they are marked **[spike]**.

---

## 0. What is wrong today (measured 9 Oct 2026)

1. **Connecting is a paste, buried in setup.**
   - The Meta token is pasted in the wizard's mandatory step 2.
   - Re-running `/setup` on a configured workspace jumps straight to the model step, so the token step can't be reached again.
   - The wizard's summary says "meta token (0 chars)" although a token is saved.
2. **Expiry is a dead end.**
   - Long-lived Meta tokens last about 60 days, and nothing records when.
   - Code 190 comes back as a raw "code Some(190)", and the fix it names ("run /setup") can't reach the token step.
   - The LeadZump token is a session token that expires too: it returned 401 on 8 Aug and again on 9 Oct.
3. **One CRM namespace for every client.**
   - `crm_keys` in `auth.json` is global, so two clients on LeadZump both write `leadzump_token` and the second silently overwrites the first.
   - This blocks "any client".
4. **Secrets are echoed.** `crm secret` (stdin `read_line`) and `crm connect` (rustyline) both show the secret on screen, and a pasted LLM key is dropped on the wizard's key step (`event.rs:216-222`).
5. **No Google Ads at all,** and no single place that says what is connected, what is expiring, and what is missing.
6. **Fixed already (9 Oct).**
   - An unreadable `auth.json` used to be treated as empty, so the next save wiped every key.
   - Writes now take a lock, and files are created 0600.
   - A CRM 401 now names the real secret ("moneyball crm secret leadzump_token") instead of a "&lt;name&gt;" placeholder.

---

## 1. What each platform allows (the constraints the design follows)

**Meta**
- **No localhost callback.** In Live mode, Facebook blocks plain-http localhost redirects. Its code-for-token exchange and its long-lived exchange both need the **app secret**, which must never ship in a binary (Platform Terms 6.a.iv).
- **Device sign-in** (`/device/login`, `/device/login_status`) needs only the app id and the client token, which Facebook documents as not secret.
  - The user types a code at facebook.com/device, so it works over SSH and on a phone.
  - Tokens last up to about 60 days, with no refresh.
  - **[spike]** It is documented, but not confirmed, for `ads_read` on a Business-type app.
- **Facebook Login for Business** with a system-user configuration issues business-owned tokens that never expire. Its code exchange needs the app secret, so it needs a small hosted broker.
- **`ads_read`** covers everything moneyball reads. `leads_retrieval` drags `ads_management`, a write scope, plus `business_management` and page scopes onto the consent screen.
- **"Any client" needs approvals.** Advanced Access for `ads_read` requires Business Verification (up to 14 business days), Access Verification, and App Review (about a week per round). Until then, only people with a role on the app can sign in.

**Google Ads**
- **A plain browser login works.** Desktop OAuth client, loopback `127.0.0.1`, PKCE S256. Google says a desktop client's secret is not treated as a secret, so the client id and secret ship in the binary.
- **There is no device flow and no copy-paste flow:** the device grant excludes the `adwords` scope, and the out-of-band flow is gone. Headless means pasting the address the browser lands on.
- **Developer tokens were retired on 9 Sep 2026.** API access now belongs to the Cloud project that owns the OAuth client.
  - Basic access needs brand verification.
  - `adwords` is a sensitive scope, so it needs verification too.
  - A project left in "Testing" gets refresh tokens that die after 7 days.
- **There is no read-only Ads scope;** the consent screen says "edit".
- **Modlix may already have the project.** Its LeadZump and AdZump services already call the Google Ads API, so an Ads-enabled Cloud project may already exist; adding a Desktop client there is the fastest route.

**CRMs**

| CRM | What it supports for a local app | moneyball's connect |
|---|---|---|
| LeadZump | Modlix session token that expires; the platform already has one-time-token SSO with a callback | Browser sign-in once Modlix adds an "Authorize moneyball" page; token paste until then |
| LeadSquared | Access and secret keys only | Open Settings > API and Webhooks, ask for the API host, masked paste of both keys |
| Freshsales | API key only | Open the API settings page, masked paste |
| HubSpot | OAuth needs the client secret; private-app tokens exist | Paste a read-scoped token (browser sign-in only with a broker) |
| Zoho CRM | OAuth, secret needed at exchange and refresh | Paste or broker |
| Pipedrive | OAuth needs the secret; personal token exists | Paste the personal token |
| Salesforce | OAuth with PKCE as a public client | Browser sign-in, after each client's admin installs the app |
| anything else | an API | Paste the "list leads" request; the AI drafts the field map once (today's path) |

---

## Part A - Core (moneyball-core)

### A1. Credentials v2 (`secrets.rs`)

| Key | Holds | Scope |
|---|---|---|
| `meta` | `token`, `method` (browser or paste), `who`, `user_id`, `scopes`, `obtained_at`, `expires_at` (null = never) | per person |
| `google` | `refresh_token`, `who`, `scopes`, `obtained_at`; access tokens are minted per pull, never stored | per person |
| `llm_keys` | as today | per person |
| `workspaces.<workspace_id>.crm_keys` | the CRM's secrets | **per client** |

- `secret:<name>` resolves the workspace's entry first, then the legacy global one.
- **Migration:** a legacy `meta_access_token` loads as `{method: paste, expires_at: null}`.
- **Stored expiry** is the earlier of `expires_at` and `data_access_expires_at`.
- **A new credential replaces the old only after it is verified:** for Meta it covers the configured ad accounts, and for a CRM the test pull passes.
- **Never log** tokens, refresh tokens, auth codes, PKCE verifiers, callback URLs or token-endpoint bodies. No agent tool may read connections.

### A2. `connections/`: status, the Prompter seam, the flows (new, no network of its own)

- `status_all(cfg)` reads `auth.json`, `crm.toml` and `config.json` only, so it is instant.
- `trait Prompter { fn show(&mut self, s: Show); fn ask(&mut self, q: Ask) -> Option<Answer>; }` is one flow with two front ends:
  - **the shell:** `println` and no-echo reads;
  - **the TUI:** a worker thread, mpsc both ways, polled on the 100 ms tick, with Esc setting an `AtomicBool`.
- `fix_for(&Error) -> Option<Provider>`: `MetaAuth` goes to meta, `CrmAuth` to crm, `LlmAuth` to model. The fix line is rendered per surface: `/connect meta` in the TUI, `moneyball connect meta` in the shell.
- The CRM orchestration moves here from the binary's `connect_flow.rs`, which closes audit package G.

### A3. Meta device sign-in (`meta.rs`; the network boundary is unchanged)

- **Run the [spike] first, for one hour:** `device/login` with `ads_read` on a Business-type app in Dev mode, signed in as a tester, then list the tester's ad accounts. If it fails, Meta v1 becomes Login for Business plus the broker (decision 2).
- `device_login_start` returns the code, the URL, `expires_in` and `interval`. `device_login_poll` handles these codes:
  - 1349174: still pending.
  - 1349172: slow down.
  - 1349152: the code expired.
  - A token: success.
- **After sign-in:** today's `list_ad_accounts`, then the picker, then a coverage check against the configured accounts.
- **v1 scope is `ads_read` only.** Meta leads stay on the pasted system-user-token path and the CRM (decision 3).

### A4. CRM connect

- Presets gain `token_page` and per-field help; LeadSquared asks for its API host.
- The live test pull gates the save. Unknown stage names are mapped once and saved in `crm.toml`; re-connecting keeps the map.
- A learned map is also kept per CRM host, with secrets stripped, in `~/.moneyball/crm-specs/<host>.toml`. The next client on the same host starts from a draft.
- **LeadZump (decision 4):** an "Authorize moneyball" page on the Modlix platform hands a one-time token to moneyball's loopback. The Modlix side needs a callback allow-list and a long-lived, ideally read-only, token for this flow.

### A5. Google Ads, phase 3

- **`connections/loopback.rs`**, the one inbound socket:
  - It binds `127.0.0.1` (never `0.0.0.0`) before the authorize URL is built.
  - `state` must match; a wrong `state` gets a 400 and the listener keeps waiting.
  - It handles one callback or 5 minutes, then closes.
  - It serves a static ASCII success page that echoes nothing.
- **`fetch/google.rs`:** `customers:listAccessibleCustomers`, then `customer_client` under managers, then `googleAds:searchStream` of daily ad-level stats.
  - `segments.date` is in the account's time zone, and cost is `cost_micros / 1e6`.
  - Performance Max spend goes into an explicit unattributed bucket.
- **The hub's Google row appears only once a Google pull lands in `/brief`.**

### A6. The network boundary

- **Phases 1 and 2: no amendment.** Device sign-in is two calls to graph.facebook.com from `meta.rs`, and CRM tests run through `crm/fetch.rs`.
- **Phase 3 wording:** "and `connections/loopback.rs`, the one INBOUND socket: a 127.0.0.1-only listener for a single browser sign-in, closed after one callback or 5 minutes; never binds 0.0.0.0." `fetch/google.rs` joins the outbound list.
- **`arch_contract` gains:**
  - an inbound-socket test that rejects `TcpListener`, `UdpSocket` and `bind(` outside `loopback.rs`. Today an inbound listener anywhere passes unnoticed, because the test only greps for "reqwest".
  - a check that no `:mutate` URL exists, since the Google scope can edit.

---

## Part B - Screens

### B1. Commands

```
moneyball connect                          status of every source; on a TTY, then asks which to connect
moneyball connect meta   [--with-token] [--no-browser]
moneyball connect crm    [--no-browser]
moneyball connect model  [--with-key]
moneyball connect google [--no-browser]    phase 3: hidden from --help until a Google pull exists
moneyball disconnect <meta|crm|model|google>
```

- **Secrets come from stdin only, never argv.**
- **`--no-browser` turns on by itself** over SSH (`SSH_CONNECTION` or `SSH_TTY`) or with no display, and no browser ever opens when stdin is not a terminal (a launchd run).
- **`MB_AGENT=1` prints JSON.**
- **TUI:** `/connect` (the hub), `/connect <provider>` and `/disconnect <provider>`.
- **Retired:**
  - `/keychain`: the hub shows model keys.
  - `crm connect`: kept as a hidden alias for one release.
  - The planned `meta token`: it is `connect meta --with-token`.
- **`crm secret` stays** as plumbing for scripts.

### B2. First run (L1, L2)

- **Workspace and model only.** The token and products steps leave the wizard, and the ad-account picker moves into `/connect meta`.
- **The paste fix:** `handle_paste` feeds the step's own insert for every character.

### B3. The hub and each flow (L3 to L14, M, X)

- **Hub:** a modal view like `/setup`, so secrets never pass through the chat composer or the session file. Three groups (ads, leads, advisor), one row per source, colour-coded: Green connected, Yellow expiring, Red ended.
- **Meta:** the code shown large, the URL, a countdown from `expires_in`, and "p paste a token instead".
- **CRM:** choose, keys from the opened settings page (masked), a test pull with a join check, the stage map, then done.
- **Failures save nothing** and name the next step. A new sign-in never replaces a working one that sees more accounts (X4).
- **Expiry:**
  - The bar and the landing warn 7 days ahead, read from `auth.json` with no network call.
  - A scheduled refresh that hits an ended sign-in records it, and the landing shows it (X3).
- **What stays in the transcript** is one summary cell per connect or disconnect, with facts only and never a secret.

### B4. Size caps

| File | Change |
|---|---|
| `setup/mod.rs` (999 of 1000) | loses the Meta and products steps (about -330 lines); lower its ceiling in the same commit |
| `render_steps.rs` | loses the same two steps |
| `commands.rs` | drops `/keychain` and gains the `/connect` dispatch |
| the binary's `main.rs` (392 of 400) | sheds its `crm` arm to `crm_cli.rs` first |
| new files | stay under 400 lines each |

---

## Phases

Each phase lists its tasks in order; each task says when it is done.

**0. Vendor setup.** Calendar-bound, so start now.
1. **Meta app** in the publisher's business, with Business Verification, Access Verification, and App Review for `ads_read` Advanced Access.
2. **Google:** a Cloud project with Ads API Basic access, plus brand and `adwords` verification.
3. **The Meta spike (A3).**

- **Done when:** a tester's device sign-in lists their ad accounts.

**1. Meta sign-in and the hub.** No boundary change.
1. Credentials v2.
2. `connections/` with the Prompter.
3. Meta device sign-in with the typed `MetaAuth`.
4. `moneyball connect` in the shell.
5. The `/connect` hub in the TUI.
6. The two-step first run.
7. The refresh's fix lines pointing to `/connect`.

- **Done when:** on the installed binary, a tester connects Meta locally and over `ssh localhost`; `--with-token` works; `disconnect` removes the credential; and the expiry warning shows 5 days out.

**2. CRM into connect.**
- The flow moves to core, `token_page` presets are added, the test pull gates the save, and `CrmAuth` routes to `/connect crm`.
- The LeadZump browser sign-in follows once the Modlix page exists.
- **Done when:** tmux E2E connects LeadZump in the TUI and LeadSquared from the shell over SSH with hidden input.

**3. Google Ads.**
- `loopback.rs` with the amendment and the inbound-socket test, then `fetch/google.rs`, then the hub row.
- **Done when:** a real Google Ads account's spend shows in `/brief`.

## Out of scope

- **A moneyball account or login of its own.** It is local-first; connect is per source.
- **Reusing a Claude or ChatGPT subscription login for the model.** Providers forbid it in third-party tools; model keys stay pasted.
- **Writing to any platform.** `disconnect` deletes locally. Revoking at the provider would be a write; at most it is a best-effort option, and it changes the read-only wording.
- **Meta lead retrieval for outside clients** in v1 (decision 3).

## Open decisions (defaults used above)

1. **Publisher of record.**
   - Which business owns moneyball's Meta app and its Google Cloud project? Its name appears on every client's consent screen.
   - Default: reuse Modlix's existing Ads-enabled Google Cloud project, and start Meta Business Verification and App Review now.
2. **Meta tokens.**
   - Default: device sign-in now (no server, re-sign-in about every 60 days, warned a week ahead).
   - Later: a small stateless broker for never-expiring business tokens, which also unlocks HubSpot and Zoho browser sign-in. It stores and logs nothing and sits only on the sign-in path, never the daily pull.
3. **Meta leads for outside clients.** Default: `ads_read` only; leads come from the CRM.
4. **LeadZump browser sign-in.** Default: yes. Build the "Authorize moneyball" page on the Modlix side; it is the one CRM where both ends are yours.
5. **Credential scope.** Default: Meta and Google sign-ins per person, CRM keys per workspace (per client).
6. **Disconnect.** Default: local delete only.

## Done when (on the installed binary)

- **A brand-new folder** goes through two steps to "nothing connected" (L3).
- **A tester connects Meta in the browser** (L5 to L7) and over SSH (M1, M3).
- **LeadZump or LeadSquared connects** with a passing test pull and a stage map (L8 to L11).
- **`/refresh` then exits 0**, and its failures route to `/connect` (S1, S2).
- **Two workspaces** on the same CRM keep separate keys.
- **An expired Meta sign-in** shows X3 on the landing, and `/connect meta` fixes it.
- **The gates:** `cargo test` and clippy green, `arch_contract` green (with the inbound-socket test from phase 3), and the binary reinstalled with `cargo install`.

## Sources

- **Meta:**
  - [manual flow](https://developers.facebook.com/docs/facebook-login/guides/advanced/manual-flow)
  - [login security](https://developers.facebook.com/docs/facebook-login/security)
  - [devices](https://developers.facebook.com/docs/facebook-login/for-devices)
  - [Login for Business](https://developers.facebook.com/documentation/facebook-login/facebook-login-for-business)
  - [permissions](https://developers.facebook.com/docs/permissions/)
  - [access levels](https://developers.facebook.com/docs/graph-api/overview/access-levels/)
  - [Platform Terms](https://developers.facebook.com/terms/dfc_platform_terms/)
- **Google:**
  - [native-app OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)
  - [limited-input devices](https://developers.google.com/identity/protocols/oauth2/limited-input-device)
  - [developer token change](https://developers.google.com/google-ads/api/docs/api-policy/developer-token)
  - [access levels](https://developers.google.com/google-ads/api/docs/api-policy/access-levels)
  - [sensitive scopes](https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification)
- **Standards:**
  - [RFC 8252](https://datatracker.ietf.org/doc/html/rfc8252) (OAuth for native apps)
  - [RFC 7636](https://datatracker.ietf.org/doc/html/rfc7636) (PKCE)
  - [RFC 9700](https://datatracker.ietf.org/doc/html/rfc9700) (OAuth security BCP)
- **CRMs:**
  - [LeadSquared API keys](https://help.leadsquared.com/how-do-i-obtain-api-access-keys-in-leadsquared/)
  - [HubSpot OAuth](https://developers.hubspot.com/docs/api-reference/latest/authentication/manage-oauth-tokens)
  - [Zoho device flow](https://www.zoho.com/accounts/protocol/oauth/devices/polling-request.html)
