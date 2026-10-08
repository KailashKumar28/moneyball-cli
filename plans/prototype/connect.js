// Connections redesign: sections L (the journey, first run to every source) and X (when a connection goes wrong).
// Loaded after redesign.js and merged into AUTHORED (end of file): reuses HEADER, RULE, COMPOSER, user, tool, OK,
// RUNNING, FAILED from there. Drafted by the login review panel (UX and architecture), 9 Oct 2026. Tag syntax as in redesign.js. New strings are ASCII; every glyph here is one the TUI already paints
// (wizard: ▸ ✓ • ╭╮╰╯│─, redesign: ● ◐ ✖ ⎿ ❯ ·). The /connect view is a modal View like the setup wizard:
// titles Cyan bold (styled_title), input boxes Cyan (input_box_open/close, inner width 60), hints DarkGray,
// errors Red. Rows marked TARGET are not buildable until their phase ships and must not render before then.

const ACME = '~/clients/acme/moneyball-data';
const abar = health => ` [dim]  └─ workspace: [/][gray]${ACME}[/]${health}`;
const NONE = '[dim]  ·  data [/][gray]none yet[/][dim]  ·  [/][yellow]● nothing connected[/]';
const META_ONLY = '[dim]  ·  data [/][gray]none yet[/][dim]  ·  [/][yellow]● no data yet[/]';
const FRESH = '[dim]  ·  data [/][gray]Thu 08 Oct[/][dim]  ·  CRM [/][gray]08 Oct[/][dim]  ·  [/][green]● fresh[/]';
const title = t => ` [cyan bold]${t}[/]`;
const box = (label, inner) => {
	const fill = '─'.repeat(Math.max(0, 60 - label.length - 3));
	return `   [cyan]╭ ${label} ${fill} ╮[/]\n   [cyan bold]│  > ${inner}[/]\n   [cyan]╰${'─'.repeat(60)}╯[/]`;
};
const keys = k => `   [dim]${k}[/]`;
const LOGO = `   [dim]__  __  ___  _   _ _______   ______    _    _     _[/]
   [dim]|  \\/  |/ _ \\| \\ | | ____\\ \\ / / __ )  / \\  | |   | |[/]
   [dim]| |\\/| | | | |  \\| |  _|  \\ V /|  _ \\ / _ \\ | |   | |[/]
   [dim]| |  | | |_| | |\\  | |___  | | | |_) / ___ \\| |___| |___[/]
   [dim]|_|  |_|\\___/|_| \\_|_____| |_| |____/_/   \\_\\_____|_____|[/]`;
const CODE = '            [white bold]WDJB-MJHT[/]';

const AUTHORED_CONNECT = {
	// L1 - first run asks two things: where, and which model. No token, no products.
	first_workspace: { text: `${HEADER}
 [dim]  └─ workspace: [/][gray]~/clients/acme[/][dim]  ·  first-run setup[/]
   [cyan bold]▸ 1 · workspace[/][dim]     2 · model[/]

${title('Workspace folder')}

   [gray]moneyball keeps this client's snapshots, reports and conversations here.[/]
   [gray]The folder is created if it does not exist.[/]

${box('workspace folder', `${ACME}█`)}

   [dim]Next you pick the AI model. Ad accounts and the CRM are connected after that, from inside moneyball.[/]


${keys('enter next  ·  esc exit setup  ·  ctrl+c quit')}` },

	// L2 - the model key, pasted (today the paste is dropped on this step: event.rs handle_paste has no step-4 arm)
	first_model_key: { text: `${HEADER}
 [dim]  └─ workspace: [/][gray]${ACME}[/][dim]  ·  first-run setup[/]
   [dim]  1 · workspace   [/][cyan bold]▸ 2 · model[/]

   [green]✓[/][gray] 1 · workspace         ${ACME}[/]
   [green]✓[/][gray]   · provider          MiniMax[/]

${title('MiniMax API key')}

   [gray]Paste the key from the API keys page of your MiniMax account.[/]
   [gray]It is stored in ~/.moneyball/auth.json, readable only by you, and sent only to MiniMax.[/]

${box('api key', '••••••••••••••••••••••••••••••••••••••••••••••••+█')}
   [dim](64 chars, pasted)[/]


${keys('enter save and pick the model  ·  esc back to providers  ·  ctrl+c quit')}` },

	// L3 - a brand-new client after setup: the advisor works, there is no data, one next step
	landing_new: { text: `${HEADER}
${abar(NONE)}

${LOGO}

   [bold]Setup done.[/][gray] The advisor is ready; it has no ad data yet.[/]

   [gray]  next:  [/][cyan bold]/connect[/][gray]   sign in to Meta Ads in your browser, then connect your CRM (about 3 minutes)[/]
   [dim]         moneyball reads spend from your ad accounts and leads from your CRM. It never changes either.[/]

${COMPOSER()}` },

	// L4 - the hub with nothing connected. Google's row is TARGET (hidden until the Google pull ships, CLAUDE.md rule).
	hub_empty: { text: `${HEADER}
${abar(NONE)}

${title('Connections')}

   [gray]moneyball reads from these. It never changes your ads or your CRM.[/]

   [dim]ads[/]
 [cyan bold]▸ Meta Ads        [/][dim]not connected    [/][gray]sign in with Facebook in your browser[/]
   [white]Google Ads      [/][dim]not connected    [/][gray]sign in with Google in your browser[/]
   [dim]leads[/]
   [white]CRM             [/][dim]not connected    [/][gray]LeadZump, LeadSquared, or any CRM with an API[/]
   [dim]advisor[/]
   [white]model           [/][green]connected        [/][gray]MiniMax, MiniMax-M3[/]


${keys('enter connect  ·  up/down move  ·  esc back to chat')}` },

	// L5 - Meta: browser opened, code on screen, waiting with the code's own expiry (device login, no local server)
	meta_waiting: { text: `${HEADER}
${abar(NONE)}

${title('Connect Meta Ads')}

   [gray]1  Your browser opened at  [/][cyan]https://www.facebook.com/device[/]
   [gray]2  Sign in as someone who can see this client's ad accounts in Ads Manager.[/]
   [gray]3  Enter this code:[/]

${CODE}

   [cyan]◐[/][gray] waiting for Facebook  ·  [/][dim]code expires in 6:41[/]

   [dim]moneyball asks only for ads_read: it reads ads and results and cannot change them.[/]
   [dim]Browser didn't open? Type that address on any device; your phone works too.[/]


${keys('esc cancel  ·  p paste a token instead')}` },

	// L6 - Meta: signed in, choose ad accounts (the wizard's picker, moved here)
	meta_accounts: { text: `${HEADER}
${abar(NONE)}

${title('Connect Meta Ads')}[dim]   [/][green]✓[/][gray] signed in as Priya Sharma[/]

   [gray]Choose the ad accounts to analyse. Each one becomes a product, named after the account.[/]
   [dim]3 of 6 chosen[/]

 [cyan bold]▸ [x] Acme Homes - Whitefield      act_2087011578504572    active[/]
   [gray][x] Acme Homes - Sarjapur        act_852565919728055     active[/]
   [gray][x] Acme Villas                  act_1043714050577651    active[/]
   [gray][ ] Acme Brand                   act_405885579167395     active[/]
   [dim][ ] Acme Launch 2024             act_311902556120044     disabled[/]
   [dim][ ] Test account                 act_771293847560        unsettled[/]


${keys('space toggle  ·  a all or none  ·  enter confirm  ·  esc back')}` },

	// L7 - Meta: success, back on the hub with what was granted and until when; cursor moves to the next gap
	meta_done: { text: `${HEADER}
${abar(META_ONLY)}

${title('Connections')}

   [green]✓ Meta Ads connected as Priya Sharma.[/]
   [gray]    ad accounts   3 of 6: Acme Homes - Whitefield, Acme Homes - Sarjapur, Acme Villas[/]
   [gray]    products      named after the accounts (rename them in /setup)[/]
   [gray]    permission    ads_read, read-only[/]
   [gray]    valid until   Mon 07 Dec 2026 (59 days). Facebook asks for a new sign-in then; moneyball warns 7 days ahead.[/]
   [gray]    next          [/][cyan bold]/refresh[/][gray] pulls the last 28 days (about a minute), or connect your CRM below[/]

   [dim]ads[/]
   [white]Meta Ads        [/][green]connected        [/][gray]Priya Sharma, 3 ad accounts, until 07 Dec[/]
   [white]Google Ads      [/][dim]not connected    [/][gray]sign in with Google in your browser[/]
   [dim]leads[/]
 [cyan bold]▸ CRM             [/][dim]not connected    [/][gray]LeadZump, LeadSquared, or any CRM with an API[/]
   [dim]advisor[/]
   [white]model           [/][green]connected        [/][gray]MiniMax, MiniMax-M3[/]

${keys('enter connect  ·  up/down move  ·  esc back to chat')}` },

	// L8 - what stays in the transcript after esc: one tool cell, no secrets
	meta_cell: { text: `${HEADER}
${abar(META_ONLY)}

${user('/connect meta', '10:02:14')}

${tool(OK, 'connect meta')}
[gray]       Meta Ads connected as Priya Sharma: 3 ad accounts, ads_read, valid until Mon 07 Dec 2026[/]
[gray]       products: Acme Homes - Whitefield, Acme Homes - Sarjapur, Acme Villas[/]
[gray]       next:  [/][cyan bold]/refresh[/][gray]   pulls the last 28 days from Meta[/]
[green]       (41.3s)[/]

${COMPOSER()}` },

	// L9 - Meta: paste instead (system user tokens, unapproved-app fallback). Never echoed, never in argv.
	meta_paste: { text: `${HEADER}
${abar(NONE)}

${title('Connect Meta Ads')}[dim]  ·  paste a token[/]

   [gray]For a system user token: Meta Business Settings > Users > System users > Generate new token,[/]
   [gray]with ads_read. Those can be set to never expire.[/]

${box('meta token', '••••••••••••••••••••••••••••••••••••••••••••••••+█')}
   [dim](231 chars, pasted)[/]

   [dim]Stored in ~/.moneyball/auth.json, readable only by you. Sent only to Facebook.[/]


${keys('enter check it with Facebook  ·  esc back to browser sign-in')}` },

	// L10 - Meta over SSH: the same code flow; nothing is opened locally
	meta_ssh: { text: `${HEADER}
${abar(NONE)}

${title('Connect Meta Ads')}

   [gray]This is an SSH session, so no browser was opened here.[/]
   [gray]1  On your laptop or phone, open  [/][cyan]https://www.facebook.com/device[/]
   [gray]2  Sign in as someone who can see this client's ad accounts in Ads Manager.[/]
   [gray]3  Enter this code:[/]

${CODE}

   [cyan]◐[/][gray] waiting for Facebook  ·  [/][dim]code expires in 6:58[/]


${keys('esc cancel  ·  p paste a token instead')}` },

	// L11 - the shell: same flow, plain lines; status is the bare command
	cli_connect_meta: { kind: 'shell', cmd: 'moneyball connect meta', text: `
[dim]moneyball-data[/] $ moneyball connect meta
No browser here (SSH session). On any device, open https://www.facebook.com/device
and enter this code:  WDJB-MJHT
waiting for Facebook (code expires in 7:00, ctrl+c cancels) ...
signed in as Priya Sharma
ad accounts this sign-in can see:
  1  Acme Homes - Whitefield   act_2087011578504572  active
  2  Acme Homes - Sarjapur     act_852565919728055   active
  3  Acme Villas               act_1043714050577651  active
  4  Acme Brand                act_405885579167395   active
analyse which? (like 1,2,3 or all): 1,2,3
Meta Ads connected: 3 ad accounts, ads_read, valid until Mon 07 Dec 2026 (59 days)
stored in ~/.moneyball/auth.json (0600)
next: moneyball refresh
[dim]moneyball-data[/] $ moneyball connect
connections  (workspace ${ACME})
  Meta Ads    connected      Priya Sharma, 3 ad accounts, ads_read    until 07 Dec 2026 (59 days)
  CRM         not connected  moneyball connect crm
  model       connected      MiniMax, MiniMax-M3
credentials: ~/.moneyball/auth.json (0600)
[dim]moneyball-data[/] $ moneyball connect meta --with-token < acme-reporting.token
token accepted: system user acme-reporting, sees all 3 of this workspace's ad accounts, ads_read, never expires
[dim]moneyball-data[/] $ ` },

	// L12 - Google (TARGET, phase 3): loopback sign-in; the short local link redirects to Google's long one
	google_waiting: { text: `${HEADER}
${abar(FRESH)}

${title('Connect Google Ads')}

   [gray]1  Your browser opened at Google's sign-in page.[/]
   [dim]   Didn't open? On this computer, open  [/][cyan]http://127.0.0.1:53682/start[/]
   [gray]2  Choose the Google account that can see this client's Google Ads, then allow access.[/]
   [dim]   Google's screen says the access includes editing: Google has no read-only permission for Ads.[/]
   [dim]   moneyball has no code that edits Google Ads; it only reads reports.[/]

   [cyan]◐[/][gray] waiting for Google  ·  [/][dim]stops waiting in 4:52[/]


${keys('esc cancel')}` },

	// L13 - Google (TARGET): signed in, pick customers, attach each to a product (products can span Meta and Google)
	google_accounts: { text: `${HEADER}
${abar(FRESH)}

${title('Connect Google Ads')}[dim]   [/][green]✓[/][gray] signed in as ads@acme.in[/]

   [gray]Choose the Google Ads accounts to analyse, and the product each one belongs to.[/]
   [dim]2 of 4 chosen[/]

 [cyan bold]▸ [x] Acme Homes Search       123-456-7890   via manager 111-222-3333   -> Acme Homes - Whitefield[/]
   [gray][x] Acme Homes PMax         234-567-8901   via manager 111-222-3333   -> new product[/]
   [gray][ ] Acme Villas Search      345-678-9012   via manager 111-222-3333[/]
   [dim][ ] Acme 2024               456-789-0123   cancelled[/]


${keys('space toggle  ·  left/right pick the product  ·  enter confirm  ·  esc back')}` },

	// L14 - Google (TARGET): success on the hub
	google_done: { text: `${HEADER}
${abar(FRESH)}

${title('Connections')}

   [green]✓ Google Ads connected as ads@acme.in.[/]
   [gray]    accounts      2: Acme Homes Search (in Acme Homes - Whitefield), Acme Homes PMax (new product)[/]
   [gray]    permission    Google Ads. Google's only Ads permission includes editing; moneyball never edits.[/]
   [gray]    valid until   no fixed end: moneyball renews it, and it stops if you remove moneyball at myaccount.google.com[/]
   [gray]    next          [/][cyan bold]/refresh[/][gray] pulls Google Ads next to Meta[/]

   [dim]ads[/]
   [white]Meta Ads        [/][green]connected        [/][gray]Priya Sharma, 3 ad accounts, until 07 Dec[/]
 [cyan bold]▸ Google Ads      [/][green]connected        [/][gray]ads@acme.in, 2 accounts[/]
   [dim]leads[/]
   [white]CRM             [/][green]connected        [/][gray]LeadSquared, last pull ok 07:30 today[/]
   [dim]advisor[/]
   [white]model           [/][green]connected        [/][gray]MiniMax, MiniMax-M3[/]

${keys('enter sign in again  ·  d disconnect  ·  up/down move  ·  esc back to chat')}` },

	// L15 - Google over SSH (TARGET): the long link printed at column 0 (copy keeps no indent), then paste the address back
	google_ssh: { text: `${HEADER}
${abar(FRESH)}

${title('Connect Google Ads')}

   [gray]This is an SSH session, so no browser was opened here. Two steps:[/]
   [gray]1  Open this link in your laptop's browser. Copy all of it; the line breaks are not part of it.[/]

[cyan]https://accounts.google.com/o/oauth2/v2/auth?client_id=40712-mb.apps.googleusercontent.com&redirect_uri=http%3A%2F%2F127.0.0.1%3A5368[/]
[cyan]2%2Fcallback&response_type=code&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fadwords&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8[/]
[cyan]URWbuGJSTw-cM&code_challenge_method=S256&state=af0ifjsldkj&access_type=offline&prompt=consent[/]

   [gray]2  After you allow access, the browser shows "127.0.0.1 refused to connect". That is expected.[/]
   [gray]   Copy the whole address from the address bar and paste it here:[/]

${box('address from your browser', '█')}


${keys('enter finish  ·  esc cancel')}` },

	// L16 - CRM: choose. Only real presets render; HubSpot is a TARGET example of an OAuth CRM.
	crm_choose: { text: `${HEADER}
${abar(META_ONLY)}

${title('Connect a CRM')}

   [gray]moneyball reads each lead's ad and stage, to show which ads bring qualified leads and site visits.[/]

 [cyan bold]▸ LeadZump        [/][gray]paste your API token[/]
   [white]LeadSquared     [/][gray]paste the access key and secret key from Settings[/]
   [white]HubSpot         [/][gray]sign in with HubSpot in your browser[/]
   [white]another CRM     [/][gray]paste the 'list leads' request from its API docs; the AI drafts the field map once[/]


${keys('enter choose  ·  esc back')}` },

	// L17 - CRM without browser sign-in: open the exact page, paste each value, masked
	crm_keys: { text: `${HEADER}
${abar(META_ONLY)}

${title('Connect LeadSquared')}

   [gray]LeadSquared has no browser sign-in for apps, so moneyball reads with an access key.[/]
   [gray]1  Your browser opened at LeadSquared: Settings > API and Webhooks.[/]
   [dim]   Didn't open? Sign in to LeadSquared and open that page from Settings.[/]
   [gray]2  Copy the three values on that page and paste each one here.[/]

   [gray]   API host     [/][white]api-in21.leadsquared.com[/][green]  ✓[/]
   [gray]   access key   [/][white]••••••••••••••••••••••••••••••••••••[/][dim]  (36 chars)[/][green]  ✓[/]
   [gray]   secret key   [/][cyan bold]> █[/]

   [dim]Stored in ~/.moneyball/auth.json for this workspace only, readable only by you. Sent only to LeadSquared.[/]


${keys('enter next  ·  esc back')}` },

	// L18 - CRM: live test pull gates the save; unknown stages mapped once and remembered in crm.toml
	crm_map: { text: `${HEADER}
${abar(META_ONLY)}

${title('Connect LeadSquared')}[dim]   [/][green]✓[/][gray] keys accepted[/]

   [gray]Test pull, last 7 days:[/]
   [green]  ✓[/][gray] 412 leads in 1 page[/]
   [green]  ✓[/][gray] 356 carry a Meta ad id; 97% of those match this workspace's ads[/]
   [yellow]  ![/][gray] 56 have no ad id (organic or walk-in): kept out of the ad funnel[/]

   [gray]Your CRM uses 4 stage names moneyball doesn't know. Map each one:[/]

 [cyan bold]▸ "Site Visit Done"      ->  Visit[/]
   [gray]"Booking Confirmed"    ->  Booking[/]
   [gray]"Not Interested"       ->  Lost[/]
   [gray]"Call Back Later"      ->  keep as a lead (never counted as qualified)[/]

   [dim]Saved in .moneyball/crm.toml with this connection: signing in again later keeps this map.[/]

${keys('up/down move  ·  left/right change  ·  enter save  ·  esc back')}` },

	// L19 - CRM: success on the hub
	crm_done: { text: `${HEADER}
${abar(META_ONLY)}

${title('Connections')}

   [green]✓ CRM connected: LeadSquared.[/]
   [gray]    test pull     412 leads in the last 7 days, 97% matched to your ads[/]
   [gray]    stages        4 mapped, saved in .moneyball/crm.toml[/]
   [gray]    keys          stored for this workspace in ~/.moneyball/auth.json[/]
   [gray]    next          [/][cyan bold]/refresh[/][gray] pulls Meta, then CRM, then writes yesterday's report[/]

   [dim]ads[/]
   [white]Meta Ads        [/][green]connected        [/][gray]Priya Sharma, 3 ad accounts, until 07 Dec[/]
   [white]Google Ads      [/][dim]not connected    [/][gray]sign in with Google in your browser[/]
   [dim]leads[/]
 [cyan bold]▸ CRM             [/][green]connected        [/][gray]LeadSquared, test pull ok just now[/]
   [dim]advisor[/]
   [white]model           [/][green]connected        [/][gray]MiniMax, MiniMax-M3[/]

${keys('enter replace the keys (keeps the stage map)  ·  d disconnect  ·  esc back to chat')}` },

	// L20 - CRM in the shell, over SSH: hidden prompts (today crm secret and crm connect echo the secret)
	cli_connect_crm: { kind: 'shell', cmd: 'moneyball connect crm', text: `
[dim]moneyball-data[/] $ moneyball connect crm
connect which CRM?
  1  LeadZump      paste your API token
  2  LeadSquared   paste the access key and secret key from Settings
  3  another CRM   paste the 'list leads' request from its API docs
> 2
No browser here (SSH session). Sign in to LeadSquared on any device and open
Settings > API and Webhooks, then paste the three values from that page.
API host: api-in21.leadsquared.com
access key (hidden):
secret key (hidden):
test pull, last 7 days: 412 leads in 1 page, 97% matched to your ads
4 stage names are new. map each: 1 Lost  2 NonContactable  3 Contactable  4 Visit  5 Revisit  6 Booking  0 keep as a lead
"Site Visit Done" -> 4
"Booking Confirmed" -> 6
"Not Interested" -> 1
"Call Back Later" -> 0
CRM connected: LeadSquared. stage map saved in .moneyball/crm.toml; keys stored for this workspace.
next: moneyball refresh
[dim]moneyball-data[/] $ ` },

	// L21 - the hub when everything is connected and one sign-in is close to its end
	hub_expiring: { text: `${HEADER}
${abar('[dim]  ·  data [/][gray]Thu 08 Oct[/][dim]  ·  CRM [/][gray]08 Oct[/][dim]  ·  [/][yellow]● Meta sign-in ends in 5 days[/]')}

${title('Connections')}

   [gray]moneyball reads from these. It never changes your ads or your CRM.[/]

   [dim]ads[/]
 [cyan bold]▸ Meta Ads        [/][yellow]ends in 5 days   [/][gray]Priya Sharma, 3 ad accounts. Sign in again before Wed 14 Oct.[/]
   [white]Google Ads      [/][green]connected        [/][gray]ads@acme.in, 2 accounts[/]
   [dim]leads[/]
   [white]CRM             [/][green]connected        [/][gray]LeadSquared, last pull ok 07:30 today[/]
   [dim]advisor[/]
   [white]model           [/][green]connected        [/][gray]MiniMax, MiniMax-M3[/]


${keys('enter sign in again  ·  d disconnect  ·  up/down move  ·  esc back to chat')}` },

	// X1 - Meta: the code ran out (also what an app not yet open to this Facebook user looks like from here)
	meta_timeout: { text: `${HEADER}
${abar(NONE)}

${title('Connect Meta Ads')}

   [red]✖ The code expired before Facebook confirmed it.[/][gray] Nothing was saved.[/]

   [gray]  If Facebook said "This app isn't available", moneyball's Meta app is not open to your[/]
   [gray]  Facebook account yet. Paste a system user token instead (p), or ask whoever set up moneyball[/]
   [gray]  to add you as a tester of its Meta app.[/]


${keys('enter get a new code  ·  p paste a token instead  ·  esc back')}` },

	// X2 - Meta: signed in but ads_read was switched off on Facebook's screen
	meta_denied: { text: `${HEADER}
${abar(NONE)}

${title('Connect Meta Ads')}

   [red]✖ Facebook signed you in, but moneyball was not allowed to read ads.[/][gray] Nothing was saved.[/]

   [gray]  moneyball needs ads_read to see spend and leads. On Facebook's screen, leave the ads permission on.[/]


${keys('enter try again  ·  esc back')}` },

	// X3 - Meta: the wrong Facebook account (sees no ad accounts)
	meta_wrong_account: { text: `${HEADER}
${abar(NONE)}

${title('Connect Meta Ads')}

   [yellow]! Signed in as Rahul Mehta, but this Facebook account can't see any ad accounts.[/][gray] Nothing was saved.[/]

   [gray]  Sign in as someone who runs this client's ads. Facebook keeps you signed in, so log out at[/]
   [gray]  facebook.com first. Or give Rahul access in Meta Business Settings > Accounts > Ad accounts.[/]


${keys('enter try again  ·  esc back')}` },

	// X4 - Meta re-sign-in by someone who sees less: the old sign-in is never replaced silently
	meta_relogin_mismatch: { text: `${HEADER}
${abar('[dim]  ·  data [/][gray]Thu 08 Oct[/][dim]  ·  CRM [/][gray]08 Oct[/][dim]  ·  [/][yellow]● Meta sign-in ends in 5 days[/]')}

${title('Connect Meta Ads')}[dim]  ·  signing in again[/]

   [yellow]! Signed in as Rahul Mehta, who can't see 2 of this workspace's 3 ad accounts:[/]
   [gray]    Acme Homes - Sarjapur   act_852565919728055[/]
   [gray]    Acme Villas             act_1043714050577651[/]
   [gray]  Your old sign-in (Priya Sharma, until Wed 14 Oct) is still in place.[/]


${keys('enter sign in as someone else  ·  k keep Rahul anyway (those 2 get no data)  ·  esc cancel, keep Priya')}` },

	// X5 - Google (TARGET): cancelled on Google's screen (the redirect carries error=access_denied)
	google_denied: { text: `${HEADER}
${abar(FRESH)}

${title('Connect Google Ads')}

   [red]✖ You chose Cancel on Google's screen.[/][gray] Nothing was saved.[/]


${keys('enter try again  ·  esc back')}` },

	// X6 - Google (TARGET): no redirect within 5 minutes; the remote-session hint is the likely cause
	google_timeout: { text: `${HEADER}
${abar(FRESH)}

${title('Connect Google Ads')}

   [red]✖ Google didn't come back within 5 minutes.[/][gray] Nothing was saved.[/]

   [gray]  If your browser shows "127.0.0.1 refused to connect", moneyball runs on another machine:[/]
   [gray]  press m and paste that page's address instead.[/]


${keys('enter try again  ·  m paste the address  ·  esc back')}` },

	// X7 - Google (TARGET): a Google account with no Google Ads access
	google_no_ads: { text: `${HEADER}
${abar(FRESH)}

${title('Connect Google Ads')}

   [yellow]! Signed in as kailash@gmail.com, which has no Google Ads accounts.[/][gray] Nothing was saved.[/]

   [gray]  Try again and pick the account that runs this client's ads in Google's account chooser.[/]


${keys('enter try again  ·  esc back')}` },

	// X8 - CRM: the live test refused the token; the previous connection is untouched
	crm_refused: { text: `${HEADER}
${abar(FRESH)}

${title('Connect LeadZump')}[dim]  ·  replacing the token (the stage map stays)[/]

   [red]✖ LeadZump refused this token (HTTP 401).[/][gray] Nothing was saved; the previous token is still in place.[/]

   [gray]   API token    [/][cyan bold]> █[/]

   [dim]Paste the token again, with no spaces at either end.[/]


${keys('enter test again  ·  esc back')}` },


	// X10 - the morning after a scheduled run hit the expiry: the landing says so before any number
	landing_signin_ended: { text: `${HEADER}
${abar('[dim]  ·  data [/][yellow]Tue 13 Oct[/][dim]  ·  CRM [/][yellow]13 Oct[/][dim]  ·  [/][red]● Meta sign-in ended[/]')}

   [red bold]✖ This morning's 07:30 refresh stopped:[/][gray] Facebook ended moneyball's sign-in on Wed 14 Oct.[/]
   [gray]  Your data is from Tue 13 Oct. Nothing new is pulled until you sign in again.[/]

   [gray]  next:  [/][cyan bold]/connect meta[/][gray]   sign in again in your browser (about a minute)[/]
   [gray]         [/][cyan bold]/refresh[/][gray]        then catch up today[/]

${COMPOSER()}` },

	// X11 - expiry warning, 7 days ahead, on an otherwise fresh landing (R5 plus one notice)
	landing_expiring: { text: `${HEADER}
${abar('[dim]  ·  data [/][gray]Thu 08 Oct[/][dim]  ·  CRM [/][gray]08 Oct[/][dim]  ·  [/][yellow]● Meta sign-in ends in 5 days[/]')}

   [bold]Thu 08 Oct[/][dim]  ·  refreshed 07:30 this morning (scheduled, 1m 18s)[/]
[gray]   spend Rs.98,412  ·  Meta leads 47  ·  CRM leads 44  ·  qualified 9  ·  Rs.10,935 per qualified[/]
[gray]   report  [/][cyan]${ACME}/.moneyball/reports/2026-10-09/Acme-Daily-08-Oct-2026.html[/]

   [yellow]! Facebook ends moneyball's Meta sign-in on Wed 14 Oct (5 days).[/][gray] After that the morning refresh stops.[/]
   [gray]  Signing in again takes a minute and keeps your products and settings.  next: [/][cyan bold]/connect meta[/]

${COMPOSER()}` },

	// X12 - disconnect: local delete only (revoking at Meta would be a write to Meta)
	disconnect_cell: { text: `${HEADER}
${abar('[dim]  ·  data [/][gray]Thu 08 Oct[/][dim]  ·  CRM [/][gray]08 Oct[/][dim]  ·  [/][yellow]● Meta not connected[/]')}

${user('/disconnect meta', '11:20:05')}

${tool(OK, 'disconnect meta')}
[gray]       Meta Ads disconnected: the sign-in was deleted from ~/.moneyball/auth.json.[/]
[gray]       Snapshots, reports and products stay. Facebook still lists moneyball under[/]
[gray]       Settings > Business integrations until you remove it there.[/]
[gray]       reconnect: [/][cyan bold]/connect meta[/]
[green]       (3ms)[/]

${COMPOSER()}` },
};

Object.assign(AUTHORED, AUTHORED_CONNECT);
