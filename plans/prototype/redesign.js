// The redesign's screens, authored in the TUI's own cells: "Daily refresh and data health" (sections R and S on
// the board). Styles copy the captured frames run for run (header Cyan bold, context bar DarkGray with Gray values,
// tool cells "⎿ ●", results Gray, durations Green, composer DarkGray); only what the redesign adds is new.
// Tags: [cyan], [green], [yellow], [red], [gray] (ratatui Gray), [dim] (DarkGray), [white], [bold], [italic].
// New strings stay ASCII (AGENTS.md); the glyphs are the ones the TUI already paints.

const WS = '/Users/Kailash/DEV/moneyball-test/moneyball-data';
const RULE = ' [dim]' + '─'.repeat(134) + '[/]';
// row 1 stays blank, as in the real TUI (the first newline is the template's own)
const HEADER = `

   [cyan bold]◆[/] [cyan bold]MONEYBALL[/][dim]  ·  [/][gray]read-only Meta-ads advisor[/]
`;
// - the context bar: the workspace, then the data's health where "● ready" used to be
const bar = health => ` [dim]  └─ workspace: [/][gray]~/DEV/moneyball-test/moneyball-data[/]${health}`;
const COMPOSER = (status = '') => `${RULE}
 [cyan bold]❯ [/][cyan]█[/][italic dim]ask moneyball about your portfolio or type / for commands[/]
 [dim]  ↵ send  ·  esc clear input  ·  ⇥ complete  ·  ↑↓ scroll  ·  /exit to quit[/]${status ? `\n   ${status}` : ''}`;
const BUSY = status => `${RULE}
 [cyan bold]❯ [/][cyan]█[/][italic dim]ask moneyball about your portfolio or type / for commands[/]
 [dim]  esc cancel refresh (after the current step)  ·  other commands still work[/]
   ${status}`;
const user = (text, at) => ` [cyan bold]❯ [/][white]${text}[/]\n [dim]    ${at}[/]`;
const tool = (glyph, name) => ` [dim]  ⎿ [/]${glyph}[dim] moneyball [/][cyan bold]${name}[/]`;
const OK = '[green]●[/]';
const RUNNING = '[yellow]◐[/]';
const FAILED = '[red]✖[/]';

const AUTHORED = {
	// R1 - opening moneyball after ten quiet days: the bar says so before any number does
	landing_stale: { text: `${HEADER}
${bar('[dim]  ·  data [/][yellow]27 Sep, pulled 10 days ago[/][dim]  ·  CRM [/][yellow]11 Aug[/][dim]  ·  [/][yellow]● stale[/]')}

   [yellow bold]! Your data is 10 days old.[/][gray] Last refresh: Mon 28 Sep, 11:41 pm (manual). 9 mornings missed, 29 Sep to 07 Oct.[/]
   [gray]  CRM is older still: last pulled 12 Aug, so CRM leads, qualified and every verdict are off until a refresh.[/]
   [gray]  Nothing refreshes on its own yet.[/]

   [gray]  next:  [/][cyan bold]/refresh[/][gray]   pulls Meta, then CRM, then writes yesterday's report (about 2 minutes)[/]
   [gray]         [/][cyan bold]/schedule[/][gray]  so it happens every morning before you open this[/]

${COMPOSER()}` },

	// R2 - one command, every step on screen while it runs
	refresh_running: { text: `${HEADER}
${bar('[dim]  ·  data [/][yellow]27 Sep, pulled 10 days ago[/][dim]  ·  CRM [/][yellow]11 Aug[/][dim]  ·  [/][cyan]◐ refreshing[/]')}

${user('/refresh', '07:31:02')}

${tool(RUNNING, 'refresh')}[dim]   for Wed 07 Oct  ·  00:52 so far[/]

[gray]       1  Meta insights          [/][green]done[/][gray]     5 accounts, 1,104 rows, 28 days to 07 Oct            [/][green](18.2s)[/]
[gray]       2  creatives and leads    [/][green]done[/][gray]     64 creatives, 1,812 leads, 33 new images             [/][green](21.0s)[/]
[gray]       3  CRM (LeadZump)         [/][yellow]running[/][gray]  page 7 of 12, 431 tickets so far[/]
[dim]       4  daily report           waits for CRM[/]

${BUSY('[cyan]◐[/][dim] refresh running  ·  step 3 of 4[/]')}` },

	// R3 - done: yesterday in one line, the report path whole, the next run
	refresh_done: { text: `${HEADER}
${bar('[dim]  ·  data [/][gray]Wed 07 Oct[/][dim]  ·  CRM [/][gray]07 Oct[/][dim]  ·  [/][green]● fresh[/]')}

${user('/refresh', '07:31:02')}

${tool(OK, 'refresh')}[dim]   for Wed 07 Oct[/]

[gray]       1  Meta insights          [/][green]done[/][gray]     5 accounts, 1,104 rows, 28 days to 07 Oct            [/][green](18.2s)[/]
[gray]       2  creatives and leads    [/][green]done[/][gray]     64 creatives, 1,812 leads, 33 new images             [/][green](21.0s)[/]
[gray]       3  CRM (LeadZump)         [/][green]done[/][gray]     12 pages, 655 ad tickets, 97% of Meta leads joined   [/][green](41.6s)[/]
[gray]       4  daily report           [/][green]done[/][gray]     Fincity-Daily-07-Oct-2026.html                       [/][green](0.7s)[/]

[bold]       Wed 07 Oct:[/][gray]  spend Rs.101,268  ·  Meta leads 51  ·  CRM leads 42  ·  qualified 7  ·  Rs.14,467 per qualified[/]
[gray]       report  [/][cyan]${WS}/.moneyball/reports/2026-10-08/Fincity-Daily-07-Oct-2026.html[/]
[dim]               o open it  ·  /brief or /funnel <product> to dig in[/]
[green]       (1m 21s)[/]

[gray]   Want this every morning?  [/][cyan bold]/schedule[/]

${COMPOSER('[dim]refreshed 07:32  ·  run record .moneyball/runs/2026-10-08.json[/]')}` },

	// R4 - one confirm installs the morning run
	schedule: { text: `${HEADER}
${bar('[dim]  ·  data [/][gray]Wed 07 Oct[/][dim]  ·  CRM [/][gray]07 Oct[/][dim]  ·  [/][green]● fresh[/]')}

${user('/schedule', '07:33:40')}

${tool(OK, 'schedule')}

[bold]       Refresh every morning at 07:30?[/]
[gray]       Pulls Meta, then CRM, then writes yesterday's report, the same as /refresh. It runs on this Mac:[/]
[gray]       if the Mac is asleep at 07:30 it runs when it wakes; if it is off, the next morning catches up.[/]

[gray]       writes   [/][white]~/Library/LaunchAgents/ai.moneyball.refresh.6f3a91c2.plist[/]
[gray]       runs     [/][white]moneyball refresh --data-root ${WS}[/]
[gray]       logs     [/][white]${WS}/.moneyball/runs/[/]
[gray]       remove   [/][white]/schedule off[/][gray]  (or moneyball schedule --off)[/]

[gray]       [/][cyan bold]y[/][gray] install at 07:30   [/][cyan bold]t[/][gray] change the time   [/][cyan bold]p[/][gray] print the plist instead (install it yourself)   [/][cyan bold]esc[/][gray] cancel[/]

${COMPOSER()}` },

	// R5 - the next morning: the run already happened, the landing says what it found
	landing_ok: { text: `${HEADER}
${bar('[dim]  ·  data [/][gray]Thu 08 Oct[/][dim]  ·  CRM [/][gray]08 Oct[/][dim]  ·  [/][green]● fresh[/][dim]  ·  next 07:30 Sat[/]')}

   [bold]Thu 08 Oct[/][dim]  ·  refreshed 07:30 this morning (scheduled, 1m 18s)[/]
[gray]   spend Rs.98,412  ·  Meta leads 47  ·  CRM leads 44  ·  qualified 9  ·  Rs.10,935 per qualified[/]
[gray]   vs Wed 07 Oct:  qualified [/][green]+2[/][gray]   cost per qualified [/][green]-Rs.3,532[/]
[gray]   report  [/][cyan]${WS}/.moneyball/reports/2026-10-09/Fincity-Daily-08-Oct-2026.html[/]
[dim]           o open it  ·  /status for the last 14 mornings[/]

${COMPOSER()}` },

	// R6 - the ledger: every morning, what came in, what failed and how it was fixed
	status: { text: `${HEADER}
${bar('[dim]  ·  data [/][gray]Thu 08 Oct[/][dim]  ·  CRM [/][gray]08 Oct[/][dim]  ·  [/][green]● fresh[/][dim]  ·  next 07:30 Sat[/]')}

${user('/status', '09:02:11')}

${tool(OK, 'status')}

[dim]       data for     Meta   CRM    leads  report    ran[/]
[gray]       Thu 08 Oct   [/][green]ok[/][gray]     [/][green]ok[/][gray]     [/][green]ok[/][gray]     [/][green]ok[/][gray]        Fri 07:30 scheduled, 1m 18s[/]
[gray]       Wed 07 Oct   [/][green]ok[/][gray]     [/][green]ok[/][gray]     [/][green]ok[/][gray]     [/][green]ok[/][gray]        Thu 07:31 manual, 1m 21s[/]
[gray]       Tue 06 Oct   [/][dim]--     --     --     --        missed: nothing scheduled yet[/]
[gray]       Mon 05 Oct   [/][dim]--     --     --     --        missed: nothing scheduled yet[/]
[dim]       ... 7 more missed days, 28 Sep to 04 Oct[/]
[gray]       Sun 27 Sep   [/][green]ok[/][gray]     [/][red]none[/][gray]   [/][green]ok[/][gray]     [/][yellow]no CRM[/][gray]    Mon 23:41 manual: Meta only, crm fetch never ran[/]

[gray]       schedule   every day 07:30 IST  ·  next Sat 10 Oct 07:30  ·  /schedule off to stop[/]
[gray]       tokens     Meta valid to 19 Nov (42 days)  ·  LeadZump ok at 07:30 today[/]
[gray]       records    ${WS}/.moneyball/runs/[/]
[green]       (9ms)[/]

${COMPOSER()}` },

	// S1 - CRM failed, Meta fine: the report still ships, with CRM hidden and saying why (never zeros)
	refresh_degraded: { text: `${HEADER}
${bar('[dim]  ·  data [/][gray]Wed 07 Oct[/][dim]  ·  CRM [/][yellow]12 Aug[/][dim]  ·  [/][yellow]● CRM missing[/]')}

${user('/refresh', '07:31:02')}

${tool('[yellow]●[/]', 'refresh')}[dim]   for Wed 07 Oct  ·  finished without CRM[/]

[gray]       1  Meta insights          [/][green]done[/][gray]     5 accounts, 1,104 rows, 28 days to 07 Oct            [/][green](18.2s)[/]
[gray]       2  creatives and leads    [/][green]done[/][gray]     64 creatives, 1,812 leads, 33 new images             [/][green](21.0s)[/]
[gray]       3  CRM (LeadZump)         [/][red]failed[/][gray]   LeadZump refused the token (HTTP 401): it has expired[/]
[gray]       4  daily report           [/][yellow]no CRM[/][gray]   Fincity-Daily-07-Oct-2026.html, CRM columns and verdicts hidden[/]

[yellow]       ! The report went out without CRM.[/][gray] Meta numbers are complete; CRM leads, qualified, cost per[/]
[gray]         qualified and every Kill/Scale verdict are hidden in it, not shown as zero.[/]
[gray]       fix:  [/][cyan bold]moneyball crm secret leadzump_token[/][gray]   paste a fresh token (LeadZump > Settings > API)[/]
[gray]             then [/][cyan bold]/refresh[/][gray]   reuses this morning's Meta pull: only CRM and the report run again[/]
[green]       (41.0s)[/]

${COMPOSER('[dim]run record .moneyball/runs/2026-10-08.json  ·  outcome: degraded[/]')}` },

	// S2 - Meta refused: nothing is half-written, the fix is one command
	refresh_failed: { text: `${HEADER}
${bar('[dim]  ·  data [/][yellow]27 Sep, pulled 10 days ago[/][dim]  ·  CRM [/][yellow]11 Aug[/][dim]  ·  [/][red]● refresh failed[/]')}

${user('/refresh', '07:31:02')}

${tool(FAILED, 'refresh')}[dim]   for Wed 07 Oct  ·  stopped at step 1[/]

[gray]       1  Meta insights          [/][red]failed[/][gray]   Meta refused the access token (code 190): it expired on 03 Oct[/]
[dim]       2  creatives and leads    skipped[/]
[dim]       3  CRM (LeadZump)         skipped, so CRM stays matched to the same day as Meta[/]
[dim]       4  daily report           skipped[/]

[red]       ✖ Nothing was written today.[/][gray] Your last good data (27 Sep) is untouched.[/]
[gray]       fix:  [/][cyan bold]moneyball meta token[/][gray]   paste a new long-lived token (Business Settings > System users >[/]
[gray]             Generate token, with ads_read). Long-lived tokens last about 60 days; /status shows the date.[/]
[gray]             then [/][cyan bold]/refresh[/]
[green]       (1.9s)[/]

${COMPOSER('[dim]run record .moneyball/runs/2026-10-08.json  ·  outcome: failed[/]')}` },

	// S3 - one account fails: the other four products still refresh
	refresh_partial: { text: `${HEADER}
${bar('[dim]  ·  data [/][gray]Wed 07 Oct[/][dim]  ·  CRM [/][gray]07 Oct[/][dim]  ·  [/][yellow]● 4 of 5 products[/]')}

${user('/refresh', '07:31:02')}

${tool('[yellow]●[/]', 'refresh')}[dim]   for Wed 07 Oct  ·  one product missing[/]

[gray]       1  Meta insights          [/][yellow]4 of 5[/][gray]   Primus by Fincity: the token can't read act_405885579167395 (permission)[/]
[gray]       2  creatives and leads    [/][green]done[/][gray]     51 creatives, 1,402 leads (Primus skipped)[/]
[gray]       3  CRM (LeadZump)         [/][green]done[/][gray]     12 pages, 655 ad tickets, 96% of Meta leads joined[/]
[gray]       4  daily report           [/][green]done[/][gray]     Fincity-Daily-07-Oct-2026.html, Primus marked "no data"[/]

[yellow]       ! Primus by Fincity has no data for 07 Oct.[/][gray] Its ad account stopped sharing with this token.[/]
[gray]       fix:  in Meta Business Settings, give the system user access to act_405885579167395 again,[/]
[gray]             then [/][cyan bold]/refresh[/][gray]   (the other four products are already done)[/]
[green]       (1m 18s)[/]

${COMPOSER('[dim]run record .moneyball/runs/2026-10-08.json  ·  outcome: degraded[/]')}` },

	// S4 - the verdict gate: with no CRM, kill and scale are withheld, never computed from zeros
	funnel_gated: { text: `${HEADER}
${bar('[dim]  ·  data [/][yellow]27 Sep, pulled 10 days ago[/][dim]  ·  CRM [/][yellow]11 Aug[/][dim]  ·  [/][yellow]● stale[/]')}

${user('/funnel Disha Namma Mane', '09:14:20')}

${tool(OK, 'funnel Disha Namma Mane')}
[gray]       FUNNEL Disha Namma Mane - by adset - 7d to 27 Sep[/]
[dim]       name                              spend   meta  cpl    crm  qual  rs_per_q  verdict[/]
[gray]       NonAdvantage+ RK                  85375   56    1525   -    -     -         [/][yellow]no CRM[/]
[gray]       Pincode Ad                        50344   55    915    -    -     -         [/][yellow]no CRM[/]
[gray]       Lookalike                         25468   36    707    -    -     -         [/][yellow]no CRM[/]
[dim]       + 5 Instagram posts under Rs.1,300 each[/]
[gray]       cheapest Meta lead first: Lookalike Rs.707 < Pincode Ad Rs.915 < NonAdvantage+ RK Rs.1,525[/]

[yellow]       ! Verdicts withheld: this snapshot has no CRM data[/][gray] (last CRM pull 12 Aug). Kill and scale need[/]
[gray]         qualified counts; computing them from zero would say "kill" to every ad set that spent.[/]
[gray]       fix:  [/][cyan bold]/refresh[/][gray]   then /funnel again[/]
[green]       (6ms)[/]

${COMPOSER()}` },

	// S5 - what the scheduler runs: plain lines for a log, an exit code for a script
	cli_refresh: { kind: 'shell', cmd: 'moneyball refresh', text: `
[dim]moneyball-data[/] $ moneyball refresh
refresh for Wed 07 Oct 2026  (workspace ${WS})
  1 Meta insights        done    5 accounts, 1,104 rows                    18.2s
  2 creatives and leads  done    64 creatives, 1,812 leads                 21.0s
  3 CRM (LeadZump)       failed  HTTP 401: token expired                    1.1s
  4 daily report         no CRM  Fincity-Daily-07-Oct-2026.html             0.7s
outcome: degraded (CRM)
fix: moneyball crm secret leadzump_token   then: moneyball refresh
report: ${WS}/.moneyball/reports/2026-10-08/Fincity-Daily-07-Oct-2026.html
run record: ${WS}/.moneyball/runs/2026-10-08.json
[dim]moneyball-data[/] $ echo $?
2
[dim]moneyball-data[/] $ MB_AGENT=1 moneyball status --json | head -c 160
{"schema":"moneyball.run/1","date":"2026-10-08","outcome":"degraded","steps":[{"name":"meta","status":"ok","rows":1104},{"name":"crm",
[dim]moneyball-data[/] $ ` },
};
