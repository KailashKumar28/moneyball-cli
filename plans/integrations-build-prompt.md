# Build prompt: connect any client's ads and CRM

You are building **connect** for moneyball-cli (`/Users/Kailash/DEV/moneyball-cli`). A new client sets up a folder and an AI model, nothing else. From inside moneyball they then connect their ad platforms and their CRM in one click:

**Connect → the browser opens the platform's own sign-in → choose and Allow → moneyball finishes by itself.**

There are no codes to type and no tokens to paste, for Meta, Google, or any CRM that has a sign-in. Only CRMs with no sign-in of any kind take a key, through a guided, masked prompt.

The target design is the flow board's sections **L** (connect any client), **M** (connect without a browser here) and **X** (when a connection goes wrong) in `plans/prototype/board.html`. Each frame's **Copy for agent** gives its exact cells, every styled run with its ratatui `Style`, and the build notes; the Build plan tab orders the tasks.

Read the repo's `CLAUDE.md`, `AGENTS.md` and `ARCHITECTURE.md` first. They are binding: the network boundary, E2E first, `cargo install` is shipping, ASCII-only authored TUI strings, never advertise a command before it works.

The platform facts come from a four-engineer review panel on 9 Oct 2026, covering identity and security, Meta, Google Ads and CRMs, and UX and architecture. The one-click requirement is the product owner's, from the same day: "connect your account → opens in Chrome → gets permissions → token generated → done", with no Meta or Google token pasting.

---

## 0. What is wrong today (measured 9 Oct 2026)

1. **Connecting is a paste, buried in setup.**
   - The Meta token is pasted in the wizard's mandatory step 2.
   - Re-running `/setup` on a configured workspace jumps straight to the model step, so the token step can't be reached again.
   - The wizard's summary says "meta token (0 chars)" although a token is saved.
2. **Expiry is a dead end.**
   - Pasted user tokens last about 60 days, and nothing records when.
   - Code 190 comes back as a raw "code Some(190)", and its fix ("run /setup") can't reach the token step.
   - The LeadZump token is a session token that expires too (401 on 8 Aug and 9 Oct).
3. **One CRM namespace for every client.** `crm_keys` in `auth.json` is global, so two clients on LeadZump overwrite each other's `leadzump_token`. This blocks "any client".
4. **Secrets are echoed.** `crm secret` and `crm connect` show the secret on screen, and the wizard drops a pasted LLM key (`event.rs:216-222`).
5. **No Google Ads at all,** and no single place that says what is connected and what is missing.
6. **Fixed already (9 Oct).**
   - `auth.json` is never wiped when unreadable.
   - Writes take a lock, and files are created 0600.
   - A CRM 401 names the real secret.

---

## 1. Why one-click needs a small connect service

**Meta: the browser can't hand the token straight to moneyball.**
- **No localhost callback.** In Live mode, Facebook blocks plain-http localhost redirects, so the browser can't send its result to `127.0.0.1`.
- **The exchange needs a secret.** Facebook's code-for-token exchange needs the **app secret**, which must never ship in a binary (Platform Terms 6.a.iv).
- **The way to non-expiring tokens.** Facebook Login for Business with a system-user configuration issues **business-owned tokens that do not expire**, on exactly the ad accounts the client picks in Facebook's own dialog. Its exchange also needs the app secret.
- **So a small service sits between them.** It receives Facebook's redirect over HTTPS, exchanges the code with the secret, and hands the token to the waiting moneyball once.

**Google** could work without the service, using a loopback listener and PKCE, because Google says a desktop client's secret is not a secret. It goes through the **same service** anyway:
- **Same experience:** connecting works the same everywhere, including over SSH.
- **Simpler moneyball:** moneyball never opens an inbound socket.

**The connect service's rules**
- Stateless. It holds a session's result for at most 2 minutes, then forgets it.
- It logs no tokens, codes or query strings, and is rate-limited.
- The Meta app secret (and any CRM OAuth secrets) live in its environment and nowhere else.
- It is on the connect path only, never on the daily pull. If it is down, connecting waits; refreshes keep working.
- It is not the CLOUD_PLAN server (no database, no accounts). The Phase 2 server can absorb it later.

**The alternative, if no service may be hosted (decision 2).** A static HTTPS page relays the browser back to `127.0.0.1`, using Facebook's OIDC + PKCE flow with no secret.
- Same screens.
- Tokens last about 60 days.
- It needs a one-hour spike to confirm `ads_read` is granted in that flow.

**Approvals for "any client"** are calendar-bound, so start them now (decision 1). Until they clear, only people listed as testers can connect.
- **Meta:** Business Verification (up to 14 business days), Access Verification, and App Review for `ads_read` Advanced Access (about a week per round).
- **Google:**
  - developer tokens were retired on 9 Sep 2026, so access belongs to the Cloud project;
  - Basic access plus brand verification plus `adwords` sensitive-scope verification;
  - never ship in "Testing", where refresh tokens die after 7 days.
  - Modlix's LeadZump and AdZump services already call the Google Ads API, so an Ads-enabled project may already exist.

**CRMs**

| CRM | Connect |
|---|---|
| LeadZump | One-click browser sign-in through an "Authorize moneyball" page on the Modlix platform. Its one-time-token SSO already exists; it needs a callback allow-list and a long-lived read-only token (decision 4). |
| HubSpot, Zoho, Pipedrive | One-click OAuth through the connect service, which holds their client secrets. |
| Salesforce | One-click OAuth with PKCE, after each client's admin installs the app. |
| LeadSquared, Freshsales | No sign-in exists, so a key. moneyball opens the exact settings page and asks for the values in a masked prompt; the save waits for a live test pull. |
| anything else | Paste the "list leads" request; the AI drafts the field map once (today's path). |

---

## Part A - Core (moneyball-core)

### A1. Credentials v2 (`secrets.rs`)

| Key | Holds | Scope |
|---|---|---|
| `meta` | `token`, `business`, `ad_accounts`, `scopes`, `connected_at` (the token does not expire) | per person |
| `google` | `refresh_token`, `who`, `accounts`, `scopes`, `connected_at`; access tokens are minted per pull, never stored | per person |
| `llm_keys` | as today | per person |
| `workspaces.<workspace_id>.crm_keys` / `crm_tokens` | the CRM's credentials | **per client** |

- `secret:<name>` resolves the workspace's entry first, then the legacy global one.
- **Migration:** the legacy pasted `meta_access_token` keeps working until the client connects through the browser.
- **A new connection replaces the old only after it is verified:** for Meta it covers the configured ad accounts, and for a CRM the test pull passes.
- **Never log** tokens, codes, verifiers, session links' results or token-endpoint bodies. No agent tool may read connections.

### A2. `connections/`: status, the Prompter seam, the flows

- `status_all(cfg)` reads `auth.json`, `crm.toml` and `config.json` only, so it is instant.
- `trait Prompter { fn show(&mut self, s: Show); fn ask(&mut self, q: Ask) -> Option<Answer>; }` is one flow with two front ends:
  - **the shell:** `println`;
  - **the TUI:** a worker thread, mpsc both ways, polled on the 100 ms tick, with Esc setting an `AtomicBool`.
- `fix_for(&Error) -> Option<Provider>`: `MetaAuth` goes to meta, `CrmAuth` to crm, `LlmAuth` to model. The fix line is rendered per surface: `/connect meta` in the TUI, `moneyball connect meta` in the shell.
- The CRM orchestration moves here from the binary's `connect_flow.rs`, which closes audit package G.

### A3. `connections/service.rs`: moneyball's side of one-click

This file is the only new network module, and it is outbound only.
1. `POST <service>/sessions {provider, code_challenge}` returns `{session_id, link}`.
2. Open the link with `open` / `xdg-open`, or print it over SSH or when there is no display. The same link works on any device.
3. Poll `GET <service>/sessions/{id}` with the verifier until the user has allowed. The response is `{token, business, ad_accounts, scopes}` or `{refresh_token, ...}`, returned once.
4. Meta: the granted ad accounts become products (today's `list_ad_accounts` confirms them). Google: `listAccessibleCustomers` and the manager walk.

Hermetic tests run against a fake service: success, denied, the link expiring after 10 minutes, cancel, and a replaced connection seeing fewer accounts.

### A4. The connect service (a separate small deployment)

**Endpoints**
- `POST /sessions`
- `GET /sessions/{id}`
- `/m/{id}`: redirect to Facebook Login for Business with `config_id`.
- `/m/cb`: exchange the code with the app secret, then show the "connected" page.
- The Google pair later.

**Meta app setup**
- A Business-type app with Facebook Login for Business.
- A configuration: system-user token, never expires, assets = ad accounts, permission `ads_read`.
- The redirect URI is the service's `/m/cb` only, with Strict Mode and Enforce HTTPS on.

### A5. CRM connect

- Browser sign-in where the CRM has one (the table above). Presets gain `connect: oauth | key` and, for keys, `token_page` plus per-field help.
- The live test pull gates the save. Unknown stage names are mapped once and saved in `crm.toml`; re-connecting keeps the map.
- A learned map is kept per CRM host, with secrets stripped, in `~/.moneyball/crm-specs/<host>.toml`, and offered to the next client on that host.

### A6. Google Ads (phase 3)

- Google's OAuth on the connect service.
- `fetch/google.rs`: `googleAds:searchStream` of daily ad-level stats.
  - `segments.date` is in the account's time zone, and cost is `cost_micros / 1e6`.
  - Performance Max spend goes into an explicit unattributed bucket.
- The hub's Google row appears only once a Google pull lands in `/brief`.
- `arch_contract` forbids `:mutate` URLs, since Google has no read-only Ads scope.

### A7. The network boundary

- ARCHITECTURE section 1 gains `connections/service.rs`: outbound HTTPS to the connect service only.
- `arch_contract` gets `service.rs` in its allowed list and a new test rejecting `TcpListener`, `UdpSocket` and `bind(` anywhere. moneyball never listens.
- The read-only promise is reworded: "never mutates ad or CRM data; the only non-read calls are to the connect service and OAuth token endpoints."

---

## Part B - Screens

### B1. Commands

```
moneyball connect                  status of every source; on a TTY, then asks which to connect
moneyball connect meta             one click: opens Facebook, waits, done
moneyball connect google           phase 3: hidden from --help until a Google pull exists
moneyball connect crm              browser sign-in where the CRM has one, else a guided key
moneyball connect model            provider, key, model (LLM keys stay pasted: providers forbid reusing chat logins)
moneyball disconnect <meta|google|crm|model>
```

- No command or flag takes a Meta or Google token.
- `MB_AGENT=1` prints JSON.
- moneyball never opens a browser when stdin is not a terminal (a launchd run records the error instead).
- **TUI:** `/connect` (the hub), `/connect <provider>` and `/disconnect <provider>`.
- **Retired:** `/keychain` (the hub shows model keys), `crm connect` (kept as a hidden alias for one release), and the planned `meta token`.
- **`crm secret` stays** as plumbing for key-only CRMs and scripts.

### B2. First run (L1, L2)

- **Workspace and model only.** The token and products steps leave the wizard: ad accounts are chosen in Facebook's own dialog during connect.
- **The paste fix (model key):** `handle_paste` feeds the step's own insert for every character.

### B3. The hub and each flow (L, M, X)

- **Hub:** a modal view like `/setup`. Three groups (ads, leads, advisor), one row per source: Green connected, Red removed.
- **Meta and Google:**
  - "Facebook (Google) is open in your browser", the steps there, then "moneyball comes back here by itself".
  - `c` copies the link, Esc cancels.
  - The browser tab lands on moneyball's "connected" page.
- **Over SSH:** the link is printed and works on any device. moneyball still finishes by itself.
- **CRM:** choose, then sign in (or, for a key-only CRM, enter masked keys from the opened page), then a test pull with a join check, then the stage map, then done.
- **Failures save nothing** and name the next step. A new connection never replaces a working one that sees more accounts.
- **Access removed** in Business Settings shows on the landing after the next refresh (X3), with `/connect meta` as the fix.

### B4. Size caps

| File | Change |
|---|---|
| `setup/mod.rs` (999 of 1000) | loses the Meta and products steps (about -330 lines); lower its ceiling in the same commit |
| `commands.rs` | drops `/keychain`, adds the `/connect` dispatch |
| the binary's `main.rs` (392 of 400) | sheds its `crm` arm to `crm_cli.rs` first |
| new files | stay under 400 lines each |

---

## Phases

Each phase lists its tasks in order; each task says when it is done.

**0. Calendar and the service.** Start now.
1. **Publisher accounts:** Meta Business Verification, Access Verification and App Review for `ads_read`; the Google Cloud project's verification.
2. **The Meta app's Login for Business configuration.**
3. **The connect service** deployed on the publisher's domain.

- **Done when:** a tester clicks the link, allows in Facebook, and the service returns a token that lists their ad accounts.

**1. Meta one-click and the hub.**
1. Credentials v2.
2. `connections/` with the Prompter.
3. `service.rs`.
4. `moneyball connect` in the shell.
5. The `/connect` hub in the TUI.
6. The two-step first run.
7. The refresh's fix lines pointing to `/connect`.

- **Done when:** on the installed binary, a tester clicks Connect, allows in Chrome and lands back connected, locally and over `ssh localhost`, with nothing pasted; and `disconnect` removes the credential.

**2. CRM into connect.**
- The flow moves to core.
- LeadZump's browser sign-in, once the Modlix page exists.
- Key-only CRMs with masked prompts.
- `CrmAuth` routes to `/connect crm`.
- **Done when:** LeadZump connects in one click in the TUI, and LeadSquared connects from the shell over SSH.

**3. Google Ads.**
- OAuth on the service, then `fetch/google.rs`, then the hub row.
- **Done when:** a real Google Ads account's spend shows in `/brief`.

## Out of scope

- **A moneyball account or login of its own.** It is local-first; connect is per source.
- **Reusing a Claude or ChatGPT subscription login for the model.** Providers forbid it in third-party tools.
- **Writing to any platform.** `disconnect` deletes locally and tells the client where to remove access.
- **Meta lead retrieval for outside clients** in v1 (decision 3).

## Open decisions (defaults used above)

1. **Publisher of record and domain.**
   - Which business owns moneyball's Meta app and Google project? Its name is on every consent screen.
   - Which domain runs the connect service? `connect.moneyball.example` is a placeholder.
   - Start the verifications now.
2. **The connect service.**
   - Default: host it. You get one-click everywhere, tokens that never expire, and HubSpot/Zoho sign-in.
   - Alternative: a static relay page with 60-day Meta tokens and a spike first.
3. **Meta leads for outside clients.**
   - Default: `ads_read` only; leads come from the CRM.
   - `leads_retrieval` would add `ads_management`, a write scope, to the consent screen.
4. **LeadZump one-click.** Default: yes, build the "Authorize moneyball" page on the Modlix side.
5. **Credential scope.** Default: Meta and Google per person, CRM per workspace (per client).
6. **Disconnect.** Default: local delete only.

## Done when (on the installed binary)

- **First run:** a brand-new folder goes through two steps to "nothing connected" (L3).
- **Meta and Google, one click each:** Connect, Chrome, Allow, back in moneyball and done, with nothing typed or pasted (L5 to L7), and the same over SSH (M1).
- **LeadZump in one click,** and a key-only CRM with a passing test pull and a stage map.
- **`/refresh` then exits 0,** and its failures route to `/connect` (S1, S2).
- **Per-client keys:** two workspaces on the same CRM keep separate credentials.
- **The gates:** `cargo test` and clippy green, `arch_contract` green with the no-listener test, and the binary reinstalled with `cargo install`.

## Sources

- **Meta:**
  - [manual flow](https://developers.facebook.com/docs/facebook-login/guides/advanced/manual-flow)
  - [login security](https://developers.facebook.com/docs/facebook-login/security)
  - [Login for Business](https://developers.facebook.com/documentation/facebook-login/facebook-login-for-business)
  - [permissions](https://developers.facebook.com/docs/permissions/)
  - [access levels](https://developers.facebook.com/docs/graph-api/overview/access-levels/)
  - [Platform Terms](https://developers.facebook.com/terms/dfc_platform_terms/)
  - [OIDC + PKCE](https://developers.facebook.com/docs/facebook-login/guides/advanced/oidc-token)
- **Google:**
  - [OAuth for web servers](https://developers.google.com/identity/protocols/oauth2/web-server)
  - [developer token change](https://developers.google.com/google-ads/api/docs/api-policy/developer-token)
  - [access levels](https://developers.google.com/google-ads/api/docs/api-policy/access-levels)
  - [sensitive scopes](https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification)
- **Standards:**
  - [RFC 8252](https://datatracker.ietf.org/doc/html/rfc8252) (OAuth for native apps)
  - [RFC 7636](https://datatracker.ietf.org/doc/html/rfc7636) (PKCE)
  - [RFC 9700](https://datatracker.ietf.org/doc/html/rfc9700) (OAuth security BCP)
  - [token-mediating backend](https://datatracker.ietf.org/doc/html/draft-ietf-oauth-browser-based-apps)
- **CRMs:**
  - [LeadSquared API keys](https://help.leadsquared.com/how-do-i-obtain-api-access-keys-in-leadsquared/)
  - [HubSpot OAuth](https://developers.hubspot.com/docs/api-reference/latest/authentication/manage-oauth-tokens)
