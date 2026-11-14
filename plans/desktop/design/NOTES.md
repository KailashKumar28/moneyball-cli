# Moneyball for Mac - design mock notes

`mock.html` is one static file: every v1 screen, light and dark, at 1200x800
and 900x600. The strip above the window is mock-only (screen, size, theme).
State lives in the hash: `mock.html#funnel,dark,900` (add `,bare` to fill the
viewport with the window, which is how the screenshots are taken).

Screens (11 states for the 7 surfaces): no workspace; Today fresh; Today CRM
missing; Brief; Funnel; Funnel with no CRM; Refresh running; Refresh finished;
Report; Advisor with a tool running; Advisor streaming.

## Direction

The client report is the product's face, so the app borrows its materials and
drops its volume: the same paper, ink, serif and mono numerals, but no shadows,
no gradients, no uppercase tracked labels, and one accent used only for focus.
Tables sit on the paper as open ledgers (the report's targeting table), not in
cards. The one signature element is the report's masthead tick ruler, used
once, under the date on Today.

## Tokens

Color (light / dark). All are CSS variables on `:root`, dark under
`:root[data-theme=dark]`.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `#f4f1e8` | `#1c1a16` | window content (the report's paper) |
| `--side` | `#ebe7da` | `#161411` | sidebar |
| `--card` | `#fcfbf6` | `#25221d` | controls, number strip, user message |
| `--well` | `#ece7d9` | `#2b2823` | behind the report iframe |
| `--ink` | `#17140f` | `#ece7da` | text, primary button, table head rule |
| `--soft` | `#54503f` | `#b9b29f` | secondary text |
| `--faint` | `#766e5c` | `#958d7a` | tertiary text, table headers (4.5:1 on paper) |
| `--rule` | `#d9d2bf` | `#3a362d` | borders |
| `--hair` | `#e8e2d2` | `#2d2a24` | row lines |
| `--sel` / `--hover` | ink 9% / 4.5% | ink 11% / 5.5% | selected and hovered rows |
| `--meta` | `#2f6f9f` | `#74acd8` | Meta-sourced number dot |
| `--keep` | `#1f7a52` | `#62c196` | ok, keep, CRM-sourced number dot |
| `--gold` | `#8a5a24` | `#d3a563` | warning, no CRM, running |
| `--warn` | `#a83c26` | `#e68068` | failed, kill |
| `--focus` | `#a8763a` | `#d3a563` | keyboard focus ring, 2px, 1px offset |

Type. Three families, the report's own stacks.

| Role | Family | Size / weight |
|---|---|---|
| Day and screen headline | New York (`--serif`) | 30 / 600 (Today, empty), 20 / 600 (Refresh) |
| UI text | SF (`--sans`) | 13 / 400 body, 13 / 600 titles and selected nav |
| Advisor prose | SF | 13.5 / 1.55 |
| Table text | SF | 12.5 names (500), 11 headers (500) |
| Numbers | SF Mono (`--mono`), tabular | 12 in tables, 16 / 700 in the day strip |
| Small | SF | 12 captions, 11 meta lines |

Spacing: 4, 8, 12, 16, 24, 32 (`--s1` to `--s6`). Row height 30, header row
26, controls 26, toolbar 48, sidebar 208 (172 under 1000px), content gutter 24
(16 under 1000px), cell padding 12 (8 under 1000px).

Radii: 6 controls, 8 panels, 10 window, 10 verdict pill.

## Components

- Window: overlay title bar, traffic lights inset at 18,18, sidebar runs full
  height under them, 48px toolbar per screen.
- Sidebar: workspace button (name + folder, opens the picker), five nav rows
  with Cmd-1 to 5, health footer.
- Health footer: glyph + state + two lines (data day, CRM day). It is a
  button: it opens Refresh, and shows live progress while one runs.
- Toolbar: title, optional subtitle, right-aligned controls.
- Button (default, primary = ink fill), popup select, segmented control (real
  radio inputs).
- State glyph: check ok, cross failed, `!` warning, ring waiting, spinner
  running, dot fresh.
- Ledger table: 1px ink rule under the header, hairline rows, right-aligned
  mono numerals, left-aligned names.
- Verdict pill: keep = outlined with a dot; kill = solid fill with a cross;
  wait = no box, hollow ring, reason after a colon; no CRM = dashed outline
  with `?`. Four shapes, so it reads in grayscale.
- Day strip: the report's scorecard strip, values, source dot, change.
- Notice: gold left rule; holds the problem sentence and numbered fix steps.
- In Terminal command: label, mono command, Copy (turns to Copied).
- Steps table (Refresh): number, step, state, progress or result, time.
- Feasibility block: one sentence, then three rows.
- Report viewer: toolbar (day, window, Generate, Open in Browser) + sandboxed
  iframe.
- Advisor: session list, user message, tool row (running / done, a button so
  it can expand later), answer prose with a block caret, composer with Stop.
- Empty state: headline, one sentence, Open Workspace, terminal fallback.

## Judgment calls

1. Refresh is not a sixth nav item. It opens from the health footer and from
   Today, so the footer is the one place that says how fresh the data is and
   the place you go to change that. While a refresh runs the footer shows the
   step on every screen.
2. Today is the day's report: the notice and its fix when something is wrong,
   one line for what the last run missed (its steps are on Refresh), then the
   daily report page itself, the one a client gets, even when the data is
   stale. The fix lives on Today only; other screens point to it. (Changed
   2026-11-14 by the user's call: it was the day's number strip and a
   data-health table; the strip and the "Day strip" component are gone.)
3. No CRM is never zero. The report says so in its own scorecard. Funnel drops the CRM, qualified and cost columns
   entirely (as the report does) and every verdict is the dashed "no CRM"
   pill. I added a Funnel no-CRM state because one snapshot cannot show keep,
   kill, wait and no CRM together.
4. Blank cells, not dashes, where a ratio has no denominator (zero-spend
   product, zero-lead ad set). Real zeros stay zeros.
5. Rupee sign in the app, not "Rs.". The ASCII rule is for the TUI; the report
   already uses the sign in its tiles. The embedded report keeps its own "Rs"
   sentences untouched.
6. The embedded report stays light in dark mode. It is the client's document
   and has no dark theme; the app frames it like a PDF in Preview.
7. Sentence-case labels in the app even though the report uses tracked
   capitals. The app is read daily at 11 to 13px; capitals at that size are
   noise. The report keeps its own.
8. Primary button is ink, not a color. Color is reserved for state.
9. Verdict words are the TUI's own (keep, kill, "wait: immature", "wait: low
   spend", "no CRM").
10. Column headers are abbreviated (Qual., Rs/qual., L to Q) with a caption
    under the table, so ten Brief columns fit at 900 without wrapping.
11. Session list shows the first question and a date, not the session id.
    Titles truncate with an ellipsis at 900; this is the only truncation in
    the mock and is deliberate.
12. Stop replaces Send while a turn runs, with Esc beside it. The composer
    stays typeable. Refresh's Cancel says "Stops after the current step"
    because that is what the plan says it does.
13. Advisor tool and streaming are shown as two moments of one turn (a tool
    cannot be running while its answer streams).
14. The advisor subtitle and the empty state both carry the read-only promise.
    It is the product's one claim, so it appears where trust is decided.
15. Days control is 7 / 14 / 28 and level is Campaign / Ad set / Ad (the three
    `By` values in core). The report window choices (1 day, 7 days, month to
    date) are my guess at useful presets for `report_generate(days)`.
16. Cmd-O, Cmd-N and Cmd-1 to 5 are proposed shortcuts, not existing ones.
17. No "next refresh 07:30" line: schedule is out of v1, and the app should
    not advertise what it cannot manage.

## Content and its sources

- Today fresh, Refresh, Report: the agreed `landing_ok`, `refresh_running`,
  `refresh_done` frames (data Thu 08 Oct: spend 98,412, Meta 47, CRM 44,
  qualified 9, 10,935 per qualified; vs Wed 07 Oct +2 and -3,532).
- Today degraded and Funnel no CRM: the shipped `landing_live` and
  `funnel_nocrm_tui` captures (spend 96,869, Meta 42, HTTP 401,
  `moneyball crm secret leadzump_token`).
- Brief: the captured `/brief` (8.0 q/day, 101,014/day, 12,627/q, goal 50,
  6.3x, 2.7x). Cityville's row is derived from the totals and the advisor's
  commentary (60,509/q, 0.57/day, 4 qualified).
- Funnel: the captured `moneyball funnel "Disha Namma Mane"` rows.

Things I changed or invented, to check before sign-off:

- Brief and Funnel numbers come from the 12 Aug snapshot but are labelled "7
  days to Thu 08 Oct" so the mock tells one story.
- The real funnel capture has only "wait" verdicts. To show all four I set
  Lookalike to keep and changed Pincode Ad to 1 qualified (so 50,237 per
  qualified) to make it a rule-true kill. The advisor answer quotes those.
- The other Today deltas (spend, Meta, CRM) are computed from the two days'
  frame numbers; the TUI prints only qualified and cost.
- Report brief lines reuse the 11 Aug report's creatives with 08 Oct totals;
  "3 leads missing" is 47 minus 44.
- Session titles other than "this is very old data" are invented.
- Tool names `brief` and `funnel` in the advisor are assumed.

## Verification

`shoot.mjs` (Playwright, headless Chromium) screenshots every state in both
themes at 1200x800 and 900x600 into `shots/` (44 files plus one keyboard-focus
shot) and fails if any label, cell, button or caption wraps, clips, overflows
its container or leaves the window. Last run: OK. I also looked at all 45.

    npm i playwright && npx playwright install chromium-headless-shell
    node plans/desktop/design/shoot.mjs "$PWD/plans/desktop/design/mock.html" "$PWD/plans/desktop/design/shots"

Fixed along the way: main pane collapsed to sidebar width in the empty state;
sidebar health lines overflowed at 900; Brief table overflowed by 3px at 900;
Refresh step names were right-aligned; text and numeric headers were two
sizes; state words sat 1px off the row baseline; two notices and one caption
wrapped at 900.

Not verified: rendering in WKWebView itself (Chromium falls back from New York
to Iowan Old Style where the name is not exposed; Tauri's WebView resolves
`ui-serif` to New York), and the in-app keyboard shortcuts, which are labels
only in a static mock.
