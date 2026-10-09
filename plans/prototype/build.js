// How each redesign frame is built in moneyball-cli: where it lives, the contracts it adds, the steps that wire
// them, and how the screen goes together. The board shows it on the Frame tab and "Copy for agent" includes it.
// Checked against the code at 500e66a on 8 Oct 2026 (three-engineer review panel: journey, data/ops, advisor).

const BUILD_PROMPT = 'moneyball-cli/plans/daily-refresh-build-prompt.md and moneyball-cli/plans/integrations-build-prompt.md';
const CORE = 'crates/moneyball-core/src';
const TUI = 'crates/moneyball-tui/src';

// - one spec per part of the redesign; a frame names one or more of these
const BUILD = {
	verdicts: {
		title: 'The verdict gate (ship first)',
		where: `${CORE}/funnel/mod.rs (rows and kill), ${CORE}/report/targeting.rs (verdicts), ${TUI}/commands.rs (the agent's funnel tool)`,
		apis: [
			['Rule', 'no CRM, no verdict', 'When the snapshot has no CRM data, kill, scale and 0-CRM are withheld everywhere: funnel rows, the agent\'s tool output, the report\'s chips. Never computed from zeros.'],
			['Field', 'FunnelRow.kill: Option<bool>', 'None when CRM is absent; the table prints "no CRM" in its place.'],
			['Field', 'TargetingReport.verdicts', 'Empty, plus Verdict{code:"no_crm"} with the reason, when crm_present is false. Additive within moneyball.creative_report/1.'],
		],
		steps: [
			'Gate kill in funnel::compute on the snapshot\'s CRM presence (crm::is_empty), not on q == 0.',
			'Gate verdicts() in report/targeting.rs the same way; the exec brief already skips G2/G4 without CRM (500e66a).',
			'The agent\'s funnel tool states "verdicts withheld: no CRM data (last pull <date>)" so the model can\'t say KILL.',
			'Regenerate docs/schemas (MB_UPDATE_SCHEMAS=1 cargo test) if a field changes.',
		],
		ui: ['/funnel and the CLI funnel: the verdict column reads "no CRM" in yellow, then one line with the reason and /refresh (frame S4)', 'Report: targeting rows keep their numbers; the chip reads "no CRM" instead of Kill'],
	},
	health: {
		title: 'Data health: the context bar, the landing notice, one CRM wording',
		where: `${CORE}/crm/status.rs (CrmHealth), a new ${CORE}/health.rs (snapshot age + newest run), ${TUI}/render.rs and widgets.rs (bar), ${TUI}/app.rs (landing)`,
		apis: [
			['Type', 'CrmHealth { NoSpec, NoData, Stale { last }, Fresh { joined_pct } }', 'One source for every CRM sentence: the bar, the gate, /crm, the brief warning, the report. Replaces the NO_CRM_WARNING constant ("connect a CRM" when it is connected).'],
			['Type', 'DataHealth { day, pulled_at, crm, outcome, next_run }', 'Read from the newest snapshot and runs/<date>.json; drives the bar colour.'],
			['Rule', 'thresholds', 'Fresh: data for yesterday. Stale: yellow from 2 days, red from 7. CRM missing or older than Meta: yellow.'],
		],
		steps: ['Add CrmHealth and its note(); switch brief, funnel, report and crm status to it.', 'health::of(cfg) reads the newest snapshot dir and run record (no network).', 'Replace Status::Ready with the health segment; delete Status::Loading/Error and widgets::welcome_text (dead).', 'Landing: when not fresh, one notice cell (what, since when, the fix); when fresh, the yesterday cell (R5).'],
		ui: ['Bar: "data <day>  ·  CRM <day>  ·  ● <health>" with DarkGray labels and Gray values; health Green, Yellow, Red or Cyan (refreshing)', 'Notice: Yellow bold first sentence, Gray detail, Cyan bold commands (frame R1)'],
	},
	refresh: {
		title: 'moneyball refresh and /refresh',
		where: `a new ${CORE}/refresh.rs (orchestrator, no network of its own), ${CORE}/fetch/, ${CORE}/crm/fetch.rs, ${CORE}/report/; a new ${TUI}/refresh_flow.rs; ${crate('moneyball')}/main.rs`,
		apis: [
			['Command', 'moneyball refresh [--date D] [--force]', 'Meta, then CRM anchored to the same snapshot date, then the report. Idempotent per day: reuses today\'s Meta pull unless --force. Exit 0 ok, 2 degraded, 1 failed.'],
			['Command', '/refresh', 'The same run on a worker thread with live steps; absorbs /fetch (decision 7).'],
			['Artifact', '.moneyball/runs/<date>.json  (moneyball.run/1)', '{schema, workspace_id, run_id, trigger: schedule|manual, started_at, finished_at, outcome: ok|degraded|failed, steps: [{name, status, ms, counts, error_kind, message, fix}], artifacts}'],
			['Lock', '.moneyball/state/refresh.lock', 'One run at a time per workspace: the TUI says "scheduled refresh running since 07:30" instead of racing it.'],
		],
		steps: [
			'fetch_crm takes the target snapshot date and anchors its window to it (today: the pull date, filed into whichever dir has ads).',
			'Insights per ad account: one account failing marks that product "no data" instead of aborting every product.',
			'refresh::run(cfg, trigger, force) -> RunRecord: each step records counts, time and a typed error; the record is written even on failure.',
			'CLI: plain lines and the exit code; MB_AGENT=1 prints the run record as JSON.',
			'TUI: a worker with its own channel (not the shared app.stream slot), so the chat works while it runs; Esc cancels after the current step.',
		],
		ui: ['Tool cell "⎿ ◐ moneyball refresh" with a step table: number, name, state (done green, running yellow, failed red, skipped dim), counts, time (frames R2, R3)', 'Done: the yesterday line, the report\'s full path (wrapped, never clipped), "ctrl+o open it", the /schedule offer'],
	},
	tokens: {
		title: 'Typed errors and token recovery',
		where: `${CORE}/error.rs, ${CORE}/meta.rs, ${CORE}/fetch/mod.rs, ${CORE}/crm/fetch.rs, ${crate('moneyball')}/main.rs`,
		apis: [
			['Type', 'Error::MetaAuth { expired_on }', 'Meta code 190. Fix: moneyball meta token.'],
			['Type', 'Error::MetaPermission { account }', 'One account; the run continues with the others.'],
			['Type', 'Error::CrmAuth { secret_name }', 'HTTP 401/403. Fix: moneyball crm secret <the spec\'s real secret name> (today a literal "<name>").'],
			['Route', 'fix_for(&Error) -> Provider', 'MetaAuth -> /connect meta; CrmAuth -> /connect crm; LlmAuth -> /connect model (the connect spec). The planned "meta token" command is dropped: pasting is "connect meta --with-token".'],
		],
		steps: ['Classify Meta errors by code (190 auth, 10/200 permission) at the one Meta GET helper.', 'Every error_kind carries one exact fix command, rendered per surface (/connect in the TUI, moneyball connect in the shell); the run record stores it.', 'Remove the raw "code Some(190)" debug format (meta.rs:98).'],
		ui: ['Failed step in red with the plain reason; a "fix:" line with the command in Cyan bold; "Nothing was written today" when Meta fails (frames S1, S2)'],
	},
	status: {
		title: '/status and moneyball status',
		where: `a new ${CORE}/runs.rs (read the run records), ${TUI}/commands.rs, ${crate('moneyball')}/main.rs`,
		apis: [
			['Command', 'moneyball status [--json]', '14 rows, one per data day: Meta, CRM, leads, report, how it ran; missed days are rows. Footer: schedule, token health, records path.'],
			['Field', 'RunRecord.meta_token_expires', 'From Meta debug_token when the run validates (decision 6); shown as "Meta valid to <date>".'],
		],
		steps: ['runs::recent(cfg, 14) reads runs/*.json and fills gaps as missed days.', 'Render the same table in the TUI cell and the CLI.'],
		ui: ['DarkGray header row; ok Green, failed/none Red, no CRM Yellow, missed DarkGray (frame R6)'],
	},
	schedule: {
		title: '/schedule and moneyball schedule',
		where: `a new ${CORE}/schedule.rs (plist text, launchctl), ${TUI}/commands.rs, ${crate('moneyball')}/main.rs; AGENTS.md write boundary (decision 1)`,
		apis: [
			['Command', 'moneyball schedule [--at HH:MM] [--off] [--print]', 'Writes ~/Library/LaunchAgents/ai.moneyball.refresh.<workspace id8>.plist and bootstraps it; --print only prints it; --off removes it.'],
			['File', 'launchd plist', 'ProgramArguments: the absolute binary path, refresh, --data-root <workspace>; StartCalendarInterval Hour/Minute; WorkingDirectory the workspace; stdout/stderr into .moneyball/runs/.'],
		],
		steps: ['Confirm before any write, naming every file (frame R4).', 'launchd runs a missed calendar job when the Mac wakes; a Mac that was off catches up next morning.', 'status reads the installed plist to show "next 07:30".'],
		ui: ['A confirm cell: question in bold, what it writes/runs/logs/how to remove in Gray with White values, keys in Cyan bold'],
	},
};

function crate(name) { return `crates/${name}/src`; }

const BUILD_OF = {
	landing_stale: ['health'], refresh_running: ['refresh'], refresh_done: ['refresh'], schedule: ['schedule'], landing_ok: ['health', 'status'],
	status: ['status'], refresh_degraded: ['refresh', 'tokens'], refresh_failed: ['tokens', 'refresh'], refresh_partial: ['refresh'],
	funnel_gated: ['verdicts', 'health'], cli_refresh: ['refresh'],
	// today's frames the redesign changes
	home: ['health'], cli_funnel_nocrm: ['verdicts'], report_nocrm: ['verdicts'], cli_brief_nocrm: ['health'], nocrm_crm: ['health'],
	cli_crm_status: ['health', 'refresh'], cli_fetch_notoken: ['tokens'], setup_rerun: ['tokens'], resume: ['health'],
};

const BUILD_NOTE = {
	report_nocrm: 'Gate shipped 9 Oct: "No CRM" chips, no Kill. Since report-provenance (9 Oct) every CRM figure is hidden, not zeroed; recapture to show it.',
	cli_funnel_nocrm: 'Gate shipped 9 Oct: the kill column reads "no CRM" with the reason.',
	home: 'Today\'s frame. The bar\'s "● ready" becomes the data\'s health (R1, R5).',
	cli_fetch_notoken: 'Today\'s frame. The fix becomes "moneyball meta token" (S2).',
	setup_rerun: 'Today\'s frame. Token recovery no longer depends on reaching this step.',
	resume: 'Today\'s frame. With the health in the agent\'s state block, the advisor knows the data is stale before it plans.',
};

// ── The connect redesign (integrations-build-prompt.md): specs and the frames they serve ──

Object.assign(BUILD, {
	credentials: {
		title: 'Credentials v2 (auth.json)',
		where: `${CORE}/secrets.rs`,
		apis: [
			['Shape', 'meta: {token, business, ad_accounts, scopes, connected_at}; google: {refresh_token, who, accounts, scopes, connected_at}', 'Per person, global: one connection covers many ad accounts. No token is ever typed or pasted: both arrive through connect.'],
			['Shape', 'workspaces.<workspace_id>.crm_keys', 'Per client: two clients on LeadZump no longer overwrite each other\'s leadzump_token. secret:<name> resolves workspace first, then the legacy global entry.'],
			['Rule', 'replace only after verifying', 'A new credential replaces the old only when it covers the configured accounts (Meta) or passes the test pull (CRM).'],
		],
		steps: ['Migrate the legacy meta_access_token string (today\'s pasted token) on load; it keeps working until the client connects through the browser.', 'Hermetic test through MONEYBALL_AUTH_PATH: an old file round-trips; two workspaces keep separate CRM keys.', 'Already shipped 9 Oct: strict load (never wipe), locked writes, files created 0600.'],
		ui: ['None directly: the hub, the bar and /status read it'],
	},
	connect: {
		title: 'The connections hub: /connect and moneyball connect',
		where: `a new ${CORE}/connections/ (status, the Prompter seam, flows; no network of its own), a new ${TUI}/connect/ view, ${crate('moneyball')}/connect_cli.rs`,
		apis: [
			['Command', 'moneyball connect [meta|crm|model|google] [--with-token|--with-key] [--no-browser]', 'Bare connect = status of every source. Secrets from stdin only. --no-browser is automatic over SSH or with no display.'],
			['Command', 'moneyball disconnect <provider> / /disconnect', 'Deletes the credential locally; data stays.'],
			['Seam', 'trait Prompter { show(Show); ask(Ask) -> Option<Answer> }', 'One flow, two front ends: the shell (println + no-echo reads) and the TUI (worker thread, mpsc both ways, Esc cancels).'],
		],
		steps: ['connections::status_all(cfg) reads auth.json, crm.toml and config.json only.', 'The TUI view is modal, like /setup: secrets never enter the chat composer or the session file.', '/keychain is deleted; /setup becomes workspace + model (+ goals after the first run); the picker and the masked box move out of setup/.', 'app_state_block gains a "connections:" line so the advisor steers users to /connect.'],
		ui: ['Hub: three groups (ads, leads, advisor), one row per source with state and detail; green connected, yellow expiring, red ended (frame L4)'],
	},
	meta_signin: {
		title: 'Meta: one-click connect through the connect service',
		where: `a new ${CORE}/connections/service.rs (the only new network module: outbound HTTPS to the connect service), ${CORE}/meta.rs (accounts after connect); a separate small hosted connect service`,
		apis: [
			['Call', 'POST <service>/sessions {provider, code_challenge}', 'Returns a session id and the link to open. The link sends the browser to Facebook Login for Business (config_id: system-user token, ad accounts, ads_read).'],
			['Call', 'GET <service>/sessions/{id}?verifier=...', 'moneyball polls until the user has allowed: the service exchanged the code with the app secret and returns the token once, then forgets it (held at most 2 minutes).'],
			['Token', 'business-owned, does not expire', 'Revocable by the client in Business Settings > Integrations; a refresh that finds it removed records MetaAuth and the landing says so (frame X3).'],
		],
		steps: ['Meta app (Business type) with Facebook Login for Business and a configuration: system-user token, never expires, ad accounts, ads_read. Redirect URI on the connect service only (Strict Mode, HTTPS).', 'The connect service: stateless, stores and logs nothing beyond a session\'s 2-minute result, holds the app secret in its environment, rate-limited; never on the daily pull path.', 'moneyball opens the link (or prints it over SSH: the same link works on any device), polls on a worker, Esc cancels; then lists the granted ad accounts and makes each a product.'],
		ui: ['"Facebook is open in your browser", the three steps there, "moneyball comes back here by itself", c copy the link (frames L5 to L7, M1)'],
	},
	crm_connect: {
		title: 'CRM: connect inside moneyball',
		where: `${CORE}/connections/crm.rs (moved from the binary's connect_flow.rs), ${CORE}/crm/presets.rs (token_page, field help), ${CORE}/crm/fetch.rs (the live test)`,
		apis: [
			['Preset', 'token_page + field help', 'Opens the exact settings page where the key lives; LeadSquared also asks for its API host.'],
			['Rule', 'test pull gates the save', 'Keys and the stage map are saved only after a live pull passes; signing in again keeps crm.toml.'],
			['File', '~/.moneyball/crm-specs/<host>.toml', 'A learned field map, secrets stripped, offered as a draft to the next client on the same CRM host.'],
		],
		steps: ['Move the CRM flow into core behind the Prompter; retire "crm connect" (keep a hidden alias for one release).', 'CrmAuth routes to /connect crm, asking only for the secret that failed.', 'LeadZump: browser sign-in once Modlix adds an "Authorize moneyball" page using its one-time-token SSO (decision 4); token paste until then.'],
		ui: ['Choose, keys, test pull and stage map, done (frames L8 to L11, M4, X11)'],
	},
	google_signin: {
		title: 'Google Ads: one-click connect and the first Google pull (phase 3)',
		where: `the same connect service and ${CORE}/connections/service.rs as Meta; a new ${CORE}/fetch/google.rs`,
		apis: [
			['OAuth', 'run by the connect service', 'Google\'s web OAuth with access_type=offline; moneyball polls its session exactly as for Meta and keeps the refresh token; access tokens are minted per pull.'],
			['Call', 'customers:listAccessibleCustomers, customer_client', 'The accounts the sign-in reaches, through managers; login_customer_id stored when needed.'],
			['Call', 'googleAds:searchStream (daily ad-level GAQL)', 'segments.date in the account\'s time zone; cost_micros / 1e6; Performance Max into an unattributed bucket.'],
		],
		steps: ['Publisher setup first (decision 1): Cloud project with Ads API access (Basic), brand verification, sensitive-scope (adwords) verification; never ship in Testing (7-day refresh tokens).', 'No inbound socket in moneyball: the connect service carries the result, so over SSH it is the same link on any device.', 'Only after a Google pull lands in /brief does the hub show the Google row; arch_contract forbids :mutate URLs.'],
		ui: ['Frames L12 to L14, M5, X: TARGET until this phase ships'],
	},
});
Object.assign(BUILD_OF, {
	first_workspace: ['connect'], first_model_key: ['connect'], landing_new: ['connect', 'health'], hub_empty: ['connect'],
	meta_waiting: ['meta_signin'], meta_connected_tab: ['meta_signin'], meta_done: ['meta_signin', 'credentials'], meta_cell: ['connect'],
	meta_ssh: ['meta_signin'], cli_connect_meta: ['connect', 'meta_signin'],
	crm_choose: ['crm_connect'], crm_keys: ['crm_connect', 'credentials'], crm_map: ['crm_connect'], crm_done: ['crm_connect'], cli_connect_crm: ['connect', 'crm_connect'], crm_refused: ['crm_connect', 'credentials'],
	google_waiting: ['google_signin'], google_accounts: ['google_signin'], google_done: ['google_signin'], google_ssh: ['google_signin'], google_denied: ['google_signin'], google_timeout: ['google_signin'], google_no_ads: ['google_signin'],
	landing_signin_ended: ['tokens', 'connect'],
	meta_timeout: ['meta_signin'], meta_denied: ['meta_signin'], meta_wrong_account: ['meta_signin'], meta_relogin_mismatch: ['meta_signin', 'credentials'], disconnect_cell: ['connect', 'credentials'],
});

// ── Tasks: what is left to build, in order. Read only on the board: a task is done when VERIFIED below holds
//    an agent's evidence for it. side: 'backend' = core, 'ui' = screens. ──

const T = (id, side, owner, status, title, text, after = []) => ({ id, side, ...(Array.isArray(owner) ? { scenes: owner } : { spec: owner }), status, title, text, after });

const TASK_LIST = [
	// step 1: correctness first; nothing here waits
	T('verdict-gate', 'backend', 'verdicts', 'todo', 'Withhold verdicts without CRM', 'Kill, scale and 0-CRM are never computed from missing CRM: funnel rows, report targeting verdicts, the agent\'s funnel tool. Today the 27 Sep report carries 11 Kill chips on no data.'),
	T('crm-health', 'backend', 'health', 'todo', 'One CRM health type', 'CrmHealth {NoSpec, NoData, Stale, Fresh} with one wording; replaces NO_CRM_WARNING ("connect a CRM" when it is connected) and the wrong "today\'s snapshot" in crm status.'),
	T('typed-errors', 'backend', 'tokens', 'todo', 'Typed Meta and CRM errors', 'MetaAuth (190), MetaPermission{account}, CrmAuth{secret_name}, each routed to its /connect fix; no raw "Some(190)". (The CRM "<name>" placeholder is already gone, 9 Oct.)'),
	T('account-isolation', 'backend', 'refresh', 'todo', 'One account never aborts the pull', 'Insights per ad account: a failing account marks its product "no data"; the others land (fetch/mod.rs:79-86).'),
	T('crm-anchor', 'backend', 'refresh', 'todo', 'CRM anchored to the snapshot day', 'fetch_crm takes the target snapshot date and windows to it, so CRM always matches the same day\'s Meta pull.'),
	T('run-record', 'backend', 'refresh', 'todo', 'Run records', 'Schema moneyball.run/1 (schema/, docs/schemas), written to .moneyball/runs/<date>.json; lock at state/refresh.lock.'),
	// step 2: the run itself
	T('refresh-core', 'backend', 'refresh', 'todo', 'refresh::run', 'Meta, CRM, report in one run with a record; idempotent per day; outcome ok / degraded / failed.', ['crm-anchor', 'typed-errors', 'account-isolation', 'run-record', 'verdict-gate']),
	T('report-provenance', 'backend', ['report_top', 'report_nocrm'], 'todo', 'Report says where its data came from', 'report.json source gains meta_fetched_at, crm_fetched_at, crm status (additive in /1); the HTML\'s "pulled" stops meaning "generated"; deltas only against a report within 7 days.', ['run-record', 'crm-health']),
	T('funnel-gate-ui', 'ui', 'verdicts', 'todo', 'Show "no CRM" where a verdict was', '/funnel, CLI funnel and report chips print "no CRM" with the reason and /refresh (frame S4).', ['verdict-gate', 'crm-health']),
	T('health-bar', 'ui', 'health', 'todo', 'The context bar shows data health', 'Replace "● ready" with the data day, CRM day and health; the landing notice when not fresh; the yesterday cell when fresh (R1, R5).', ['crm-health', 'run-record']),
	// step 3: the screens on top of the run
	T('refresh-cli', 'ui', 'refresh', 'todo', 'moneyball refresh (headless)', 'Plain step lines, exit codes 0 / 2 / 1, MB_AGENT JSON (frame S5).', ['refresh-core']),
	T('refresh-tui', 'ui', 'refresh', 'todo', '/refresh in the TUI', 'Worker with live steps, done / degraded / failed / partial cells; /fetch becomes an alias (frames R2, R3, S1 to S3).', ['refresh-core']),
	T('status-cmd', 'ui', 'status', 'todo', '/status and moneyball status', '14 data days with missed days as rows, schedule and token footer (frame R6).', ['run-record']),
	T('access-health', 'backend', 'status', 'todo', 'Removed connections show up', 'A connection the client removed (MetaAuth, Google invalid_grant) shows in the bar, /status and the landing (frame X3), read from the run record.', ['credentials-v2', 'run-record']),
	// ── connect: any client's ads and CRM (integrations-build-prompt.md) ──
	T('publisher-setup', 'backend', ['meta_waiting', 'google_waiting'], 'check', 'Publisher accounts and reviews (calendar-bound: start now)', 'Meta: the business that owns moneyball\'s app, Business Verification (up to 14 business days), Access Verification, App Review for ads_read Advanced Access (about a week per round). Google: Cloud project with Ads API Basic access, brand and adwords-scope verification. Until done, only testers can sign in.'),
	T('connect-service', 'backend', ['meta_waiting', 'meta_connected_tab'], 'todo', 'The connect service', 'Small stateless HTTPS service: sessions with a PKCE-style verifier, Facebook Login for Business redirect and code exchange (app secret in its environment), Google OAuth later, the result held at most 2 minutes, nothing logged; the "connected" page. Deployed on the publisher\'s domain (decision 1).', ['publisher-setup']),
	T('credentials-v2', 'backend', 'credentials', 'todo', 'Credentials v2', 'Per-person Meta/Google sign-ins with expiry, per-workspace CRM keys, legacy migration, replace-after-verify.'),
	T('connections-core', 'backend', 'connect', 'todo', 'connections/ core + Prompter', 'Status of every source, the Prompter seam, fix_for(error) -> provider; hermetic tests with a scripted Prompter.', ['credentials-v2']),
	T('meta-connect', 'backend', 'meta_signin', 'todo', 'Meta one-click connect in moneyball', 'connections/service.rs: start a session, open or print the link, poll with the verifier, store the business token; then the granted ad accounts become products. Hermetic tests against a fake service.', ['connect-service', 'typed-errors']),
	T('connect-cli', 'ui', 'connect', 'todo', 'moneyball connect / disconnect', 'Status table, MB_AGENT JSON, open_url and SSH detection (print the link instead), no-echo prompts for key-only CRMs; crm arm moved to crm_cli.rs.', ['connections-core', 'meta-connect']),
	T('connect-tui', 'ui', 'connect', 'todo', '/connect hub in the TUI', 'Modal view, Meta flow on a worker, picker and masked box moved from setup/, /disconnect, /keychain deleted, connections line for the advisor.', ['connections-core', 'meta-connect']),
	T('wizard-two-steps', 'ui', ['first_workspace', 'first_model_key'], 'todo', 'First run: workspace + model', 'Drop the token and products steps; fix the paste root cause (handle_paste per step).', ['connect-tui']),
	T('crm-connect', 'ui', 'crm_connect', 'todo', 'CRM connect inside moneyball', 'Flow moved to core; presets gain token_page; test pull gates the save; CrmAuth routes to /connect crm; crm connect retired.', ['connections-core', 'credentials-v2']),
	T('leadzump-signin', 'backend', ['crm_choose'], 'todo', 'LeadZump browser sign-in (Modlix side)', 'An "Authorize moneyball" page on the Modlix platform: one-time token to a loopback callback, a callback allow-list, a long-lived read-only token (decision 4).', ['crm-connect']),
	T('google-signin', 'backend', 'google_signin', 'todo', 'Google Ads one-click connect and first pull (phase 3)', 'Google OAuth on the connect service; fetch/google.rs searchStream into the snapshot; the hub row only after a Google pull lands.', ['connect-service', 'connections-core']),
	T('check-connect-live', 'ui', ['meta_done', 'crm_done', 'cli_connect_meta'], 'check', 'A real client connects', 'On the installed binary, a tester clicks Connect, allows in Chrome and lands back connected, locally and over ssh localhost; LeadZump the same way; /refresh then exits 0; disconnect removes the credentials.', ['connect-cli', 'connect-tui', 'crm-connect', 'leadzump-signin']),
	// step 4: run it every morning
	T('schedule', 'ui', 'schedule', 'todo', '/schedule and moneyball schedule', 'Confirmed install of a per-workspace launchd agent, --print, --off (frame R4; decision 1 first).', ['refresh-cli']),
	// checks on the installed binary
	T('check-nocrm', 'backend', ['report_nocrm', 'cli_funnel_nocrm', 'funnel_gated'], 'check', 'No verdicts from missing CRM', 'Regenerate the 27 Sep report and run funnel on the 28 Sep snapshot: no Kill or 0-CRM anywhere, "no CRM" with the reason instead.', ['verdict-gate', 'funnel-gate-ui']),
	T('check-refresh-live', 'ui', ['refresh_done', 'cli_refresh', 'refresh_degraded'], 'check', 'A real refresh, end to end', 'Run moneyball refresh live on the test workspace: report with CRM, a valid run record, exit 0; then with the CRM secret removed: exit 2 and the right fix line.', ['refresh-cli', 'refresh-tui']),
	T('check-schedule', 'ui', ['schedule', 'landing_ok', 'status'], 'check', 'The schedule fires', 'Install for 2 minutes from now, confirm the run record appears and /status shows it, then --off removes the plist.', ['schedule', 'status-cmd']),
];

const TASK_DETAIL = {
	'verdict-gate': {
		where: `${CORE}/funnel/mod.rs:157-158, ${CORE}/report/targeting.rs verdicts(), ${TUI}/commands.rs funnel tool`,
		how: ['Pass crm presence into the kill computation; kill becomes Option<bool>', 'In targeting verdicts(): return a single no_crm verdict when the snapshot has no CRM', 'Unit tests: a snapshot with ads and no crm.json yields no kill and no Kill chip'],
		verify: ['moneyball funnel "Disha Namma Mane" on the 28 Sep snapshot shows no kill=true', 'moneyball report --date 2026-09-28 has 0 "vchip kill" in its HTML', 'cargo test and clippy green; cargo install'],
	},
	'crm-health': {
		where: `${CORE}/crm/status.rs, ${CORE}/crm/mod.rs:62 (NO_CRM_WARNING), ${CORE}/brief/mod.rs, ${CORE}/report/exec.rs`,
		how: ['CrmHealth from the spec, the secret\'s presence and the newest crm.json; note() gives the one sentence and the fix', 'crm status: the next line names the snapshot crm fetch will write into'],
		verify: ['brief, /crm and crm status say the same thing on the 28 Sep snapshot: CRM connected, not fetched for that day'],
	},
	'typed-errors': {
		where: `${CORE}/error.rs, ${CORE}/meta.rs (net_err and the GET sites), ${CORE}/crm/fetch.rs:424-429`,
		how: ['Parse Meta error.code: 190 -> MetaAuth, 10/200/294 -> MetaPermission{account}', 'CrmAuth carries the spec\'s secret name'],
		verify: ['A fake expired token (HTTP mock in a unit test) gives MetaAuth with the fix "moneyball meta token"', 'crm fetch with a wrong secret prints the real secret name'],
	},
	'account-isolation': { where: `${CORE}/fetch/mod.rs:79-86`, how: ['Collect per-account results; keep a per-product error list in FetchReport'], verify: ['A product with a bad account id leaves the other products in ads_daily.json and names the failing one'] },
	'crm-anchor': { where: `${CORE}/crm/fetch.rs:78-79, 266-285`, how: ['fetch_crm(cfg, date) windows tickets to that snapshot\'s IST day and writes into that dir only'], verify: ['crm fetch before fetch no longer lands on yesterday\'s dir'] },
	'run-record': { where: `${CORE}/schema/ (new run.rs), docs/schemas/run.v1.schema.json`, how: ['Serde struct + schemars; MB_UPDATE_SCHEMAS=1 cargo test regenerates the schema', 'Write via the atomic temp-then-rename helper'], verify: ['schema_contract test pins it'] },
	'refresh-core': { where: `${CORE}/refresh.rs (new)`, how: ['Steps: meta (skip when today\'s ads_daily.json exists unless force), creatives/leads/adsets, crm (skip if Meta failed), report', 'Outcome: failed if Meta failed; degraded if CRM failed or a product is missing; else ok'], verify: ['Unit tests per outcome with a fake snapshot dir', 'arch_contract still green: refresh.rs has no network code'] },
	'meta-token': { where: `${crate('moneyball')}/main.rs, ${CORE}/secrets.rs, ${CORE}/meta.rs`, how: ['Same stdin pattern as crm secret; validate_token before saving'], verify: ['An invalid token is refused and the old one kept'] },
	'report-provenance': { where: `${CORE}/schema/report.rs, ${CORE}/report/html.rs:52-68, ${CORE}/report/exec.rs load_prior`, how: ['source.meta_fetched_at from the snapshot\'s run record or file times', 'load_prior ignores reports older than 7 days'], verify: ['The 27 Sep report no longer compares with 11 Aug'] },
	'funnel-gate-ui': { where: `${CORE}/funnel/mod.rs table(), ${CORE}/report/sections.rs, ${TUI}/funnel_flow.rs`, how: ['Print "no CRM" in the verdict column and one reason line'], verify: ['tmux: /funnel on the 28 Sep snapshot matches frame S4'] },
	'health-bar': { where: `${TUI}/render.rs:32-38, ${TUI}/widgets.rs, ${TUI}/app.rs:155-169`, how: ['Bar from DataHealth; notice cell on launch when not fresh'], verify: ['tmux: launching on the 28 Sep snapshot matches frame R1; on a fresh one, R5'] },
	'refresh-cli': { where: `${crate('moneyball')}/main.rs`, how: ['Map outcome to exit code; MB_AGENT prints the record'], verify: ['echo $? is 0 / 2 / 1 for the three outcomes'] },
	'refresh-tui': { where: `${TUI}/refresh_flow.rs (new), ${TUI}/commands.rs COMMANDS`, how: ['Its own channel and worker; Ev per step; /fetch maps to /refresh'], verify: ['tmux: frames R2 and R3 on the real workspace; the chat answers while it runs'] },
	'status-cmd': { where: `${CORE}/runs.rs (new), ${TUI}/commands.rs, ${crate('moneyball')}/main.rs`, how: ['Gaps between run records become missed rows'], verify: ['Matches frame R6 against real run records'] },
	'token-expiry': { where: `${CORE}/meta.rs`, how: ['GET debug_token on refresh; store expires_at'], verify: ['/status shows the date; the bar warns 7 days ahead'] },
	schedule: { where: `${CORE}/schedule.rs (new)`, how: ['Plist text from a template; launchctl bootstrap gui/<uid>; --off bootouts and deletes'], verify: ['check-schedule'] },
	'check-nocrm': { where: 'the installed binary on ~/DEV/moneyball-test/moneyball-data', how: ['Regenerate, then grep the HTML'], verify: ['0 Kill chips; the funnel prints no CRM; capture.py frames F3 and F4 show the gate'] },
	'check-refresh-live': { where: 'the installed binary, live Meta and LeadZump', how: ['Run once healthy, once with the CRM secret removed'], verify: ['Run records and exit codes as described'] },
	'check-schedule': { where: 'this Mac', how: ['Install with --at two minutes ahead'], verify: ['The run record appears; --off removes the plist'] },
};

// - evidence that a task is done: '<id>': 'YYYY-MM-DD what was checked and where'. Only an agent that verified it
//   writes here; the board reads it.
const VERIFIED = {
	'verdict-gate': '2026-10-09 FunnelRow.kill is None without CRM; report verdicts() call site returns no_crm. Installed binary on ~/DEV/moneyball-test: funnel "Disha Namma Mane" (28 Sep) prints "no CRM" + reason, 12 Aug unchanged (kill false); report --date 2026-09-28 has 0 Kill, 0 zero_crm, 27 no_crm chips. Unit tests no_crm_means_no_verdict and the report_build no-CRM case.',
	'funnel-gate-ui': '2026-10-09 The one table printer (funnel/print.rs) prints "no CRM" and the reason line, so the CLI, the TUI cell and the agent tool carry it; captured from the real TUI (funnel_nocrm_tui). The /refresh pointer in frame S4 lands with /refresh (refresh-tui), never before it exists.',
	'crm-health': '2026-10-09 CrmHealth {NoSpec, NoData{last}, Fresh} with one note() in crm/status.rs; NO_CRM_WARNING deleted; target_snapshot() shared by crm fetch and crm status. Installed binary on ~/DEV/moneyball-test: moneyball brief (28 Sep) prints the note naming snapshot 2026-08-12 and "moneyball crm fetch"; crm status says "writes crm.json into snapshot 2026-09-28"; TUI /brief (captured, model off) shows the same note. Unit test crm_health_says_fetch_not_connect_when_connected.',
	'account-isolation': '2026-10-09 fetch::collect_products: a failing account marks its product "no data - <reason>", the others land; every account failing still fails the fetch (token or network). Live on a temp copy of the workspace with one account id swapped: Primus "no data - Meta 403 (#200) permission", the other four products and the snapshot written. Unit test one_failing_account_never_aborts_the_others.',
	'run-record': '2026-10-09 schema/run.rs (moneyball.run/1, pinned in docs/schemas/run.v1) and runs.rs (atomic write/read, exclusive non-blocking lock on state/refresh.lock). Unit test record_round_trips_and_the_lock_is_exclusive; the live refresh wrote runs/2026-10-09.json.',
	'refresh-core': '2026-10-09 refresh::run: Meta, assets, CRM anchored to the snapshot, report, one run record. Live on ~/DEV/moneyball-test: Meta 997 rows across 5 products, leads isolated 1 of 64 failing ads, CRM failed (LeadZump 401) with its fix, the 08 Oct report written with 0 Kill and 26 No-CRM chips, outcome degraded; a second run reused the morning Meta pull and re-ran only CRM and the report.',
	'refresh-cli': '2026-10-09 moneyball refresh [--force] [--days]: one line per step as it finishes, full reason under a failed or partial step, outcome, fix, report and run record paths; exit 2 on the live degraded run; MB_AGENT=1 prints the run record as JSON.',
	'refresh-tui': '2026-10-09 refresh_cell.rs draws the cell from structured state (R2, R3, S1 to S3): pending rows dim with "waits for", the running row yellow, done green with counts and a green (time) column, failed red and partial yellow with the reason wrapped in the detail column, skipped dim; the header says "00:52 so far", then "finished without CRM" / "stopped at step 1" / "cancelled". After it: the verdict sentence, fix: <command> with what it does, then /refresh, the day\'s numbers from report.json, the report path wrapped never clipped, ctrl+o. It runs on its own channel: chat and /funnel answered mid-refresh; the bar says "◐ refreshing", the composer "step N of 4"; Esc stopped a live --force run after Meta (CRM and report "cancelled", outcome degraded). /fetch is an unlisted alias; /brief self-heals through /refresh. Live in tmux on ~/DEV/moneyball-test at 140 and 100 columns. "Want this every morning? /schedule" closes a good run when none is installed. Not yet: live CRM page counts in the running row, "CRM (LeadZump)" naming; the fix says the shell command until /connect ships.',
	'health-bar': '2026-10-09 core health.rs (one State, the most urgent wins: no data, refresh failed, stale at 2+ days, CRM missing, N of M products, fresh) drives the bar: "data Thu 08 Oct  ·  CRM 11 Aug  ·  ● CRM missing", values yellow when behind; the path drops to its folder, then leaves the bar, before the health clips. The landing replaced "workspace configured": R1 ("Your data is 11 days old. Last refresh: Mon 28 Sep, 11:41 pm. 10 mornings missed, 29 Sep to 08 Oct." + CRM older still + next: /refresh), R5 (day, refreshed when and how, numbers, vs a report within 7 days, report, ctrl+o), plus failed (with /setup fix), CRM missing (with its fix), no data. Each captured live in tmux from real and staged workspaces. Status::Loading/Error and welcome_text deleted. The bar adds "next 16:19 Fri" while a schedule is installed; R1 adds "Nothing refreshes on its own yet." and /schedule under /refresh; R5 points at /status.',
	'status-cmd': '2026-10-09 core status.rs builds the last 14 data days (newest first) from snapshots, crm.json, leads.json, report.json and run records: ok / none / "4 of 5" / "no CRM" / --, and how each run went ("Fri 15:59 manual, 2.0s", "Meta insights failed", "stopped before it finished" for a record left running with the lock free, "missed: nothing ran"). A run of 4+ missed days shows two and folds the rest ("... 8 more missed days, 28 Sep to 05 Oct"). Footer: schedule, access (when Meta and the CRM last answered, a reused pull dated by its data, the CRM fix), records. /status draws it coloured (R6); moneyball status prints it plain; MB_AGENT=1 prints JSON. Live on ~/DEV/moneyball-test in tmux and the shell. Not yet: token expiry ("Meta valid to 19 Nov") needs the connect work. Missed days say "nothing scheduled yet" before the schedule existed and "the scheduled run never started" after.',
	'schedule': '2026-10-09 core schedule.rs: one launchd agent per workspace (ai.moneyball.refresh.<id>), plist built with HOME set (plutil -lint OK), bootstrap/bootout via launchctl, the installed time read back from the plist. /schedule shows R4 (writes / runs / logs / remove) before anything is written; t changed the time to 06:45, p printed the plist and wrote nothing, y installed at 16:19 from the TUI; launchd fired at 16:19:02 and the run record says trigger schedule (log in runs/schedule.log); /status showed "Fri 16:19 scheduled, 2.0s" and "every day 16:19 . next Sat 10 Oct 16:19"; /schedule off booted it out and deleted the plist. moneyball schedule [--at|--off|--print] did the same from the shell. AGENTS.md and ARCHITECTURE.md name the one write outside the workspace.',
	'report-provenance': '2026-10-09 report.json source gains meta_fetched_at, crm_fetched_at (from the snapshot files) and crm_status (present / not_pulled / not_connected), additive in /1, schema regenerated. The masthead says pulled at the Meta data time (08 Oct report: 03:52 pm, not the 04:19 pm render), "Meta Ads only (no CRM data for this day)" instead of claiming your CRM, and compares only with a report from the last 7 days (the 08 Oct report no longer compares with 27 Sep: "no comparable day in the last 7"). Without CRM a nocrm body class hides every CRM figure - tiles, creative-table columns 9+, targeting 5-8, project 3+, card funnel bars, the legend - with the column positions pinned by a test; the 11 Aug report with CRM renders unchanged. Client text lost its operator jargon and its plural slips ("1 qualified lead", "4 leads are missing", Rs 100,268). Screenshots of both reports checked in headless Chromium.',
	'check-nocrm': '2026-10-09 Regenerated the 27 Sep report and recaptured frames F3, F3b and F4 with capture.py: no Kill or 0-CRM anywhere on the CRM-less snapshot.',
};
