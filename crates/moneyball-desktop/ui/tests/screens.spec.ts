// One test per screen state. Each runs at 1200x800 and 900x600, light and
// dark, checks that nothing clips, overflows or wraps where it should not,
// and saves a screenshot to shots/ named like the design's
// (plans/desktop/design/shots). What a state must show is derived from the
// generated fixtures (tests/fixtures.ts), never typed in here.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Action, RefreshEvent, StepView } from "../src/ipc/types";
import { event, funnelAllVerdicts, fx, noData, refreshCrmFailed, refreshReused, todayReused } from "./fixtures";
import { emitRefresh, emitTurn, open } from "./mock";

const SIZES = [
  [1200, 800],
  [900, 600],
] as const;
const THEMES = ["light", "dark"] as const;
test.use({ timezoneId: "Asia/Kolkata" });

/**
 * The design's own check (shoot.mjs): no label, cell or button wraps, clips or leaves the window.
 * Real data relaxes it in three places, each a deliberate rule in styles.css: sentences from core
 * (.cap, .why, the step result cell, the run summary, the feasibility line) may wrap, and a name wider than its column is cut with an
 * ellipsis (td.nm and the sidebar's workspace lines; the title has the whole text).
 */
async function layoutIssues(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const out: string[] = [];
    const vis = (e: Element) => (e as HTMLElement).offsetParent !== null;
    const W = innerWidth;
    const H = innerHeight;
    const one =
      "th,td,button,label,select,h1,h2,.sub,.cap,.lbl,.why,.line,.day>*,.runhead>*,.strip>div>*,.sum p,.feas dt,.feas dd,.notice li,.stat,.sess span,.ws *,.health *,.inl,.mark";
    for (const e of document.querySelectorAll<HTMLElement>(one)) {
      if (!vis(e) || e.closest(".say,.out")) continue;
      const r = e.getBoundingClientRect();
      const cs = getComputedStyle(e);
      const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.45;
      const t = `${e.tagName.toLowerCase()}${typeof e.className === "string" && e.className ? "." + e.className : ""} "${(e.textContent ?? "").trim().slice(0, 40)}"`;
      if (r.right > W + 0.5 || r.left < -0.5) out.push("offscreen " + t);
      if (e.scrollWidth > e.clientWidth + 1 && !e.matches(".sess b,.cap,.sess button,.ws b,.ws span,td.nm")) out.push("clipped " + t);
      if (!e.matches("td,th,.notice li,.inl,.cap,.why,.sum p,.line") && r.height > lh * 1.9 && ![...e.children].some((c) => getComputedStyle(c).display !== "inline"))
        out.push("wrapped " + t);
      if (e.matches("td,th") && !e.closest("tr")?.querySelector("td.wrap") && r.height > 32) out.push("tall cell " + t);
    }
    for (const e of document.querySelectorAll<HTMLElement>(".body,.bar,.log,.side"))
      if (vis(e) && e.scrollWidth > e.clientWidth + 1) out.push("h-scroll ." + e.className);
    for (const e of document.querySelectorAll<HTMLElement>(".bar .tools,.compose")) {
      if (!vis(e)) continue;
      const r = e.getBoundingClientRect();
      if (r.right > W || r.bottom > H) out.push("out of window ." + e.className);
    }
    const side = document.querySelector<HTMLElement>(".side");
    if (side && vis(side))
      for (const e of side.querySelectorAll<HTMLElement>("b,span,.navb"))
        if (vis(e) && e.getBoundingClientRect().right > side.getBoundingClientRect().right) out.push(`sidebar overflow "${e.textContent}"`);
    return [...new Set(out)];
  });
}

/** Runs `arrange` in all four variants, then checks the layout and saves the shot. */
function state(name: string, flags: string, arrange: (page: Page) => Promise<void>, replies?: Record<string, unknown>) {
  test(name, async ({ page }) => {
    for (const [w, h] of SIZES)
      for (const theme of THEMES) {
        await page.setViewportSize({ width: w, height: h });
        await page.emulateMedia({ colorScheme: theme });
        await open(page, flags, replies);
        await arrange(page);
        expect(await layoutIssues(page), `${name} ${theme} ${w}`).toEqual([]);
        await page.screenshot({ path: `shots/${name}-${theme}-${w}x${h}.png` });
      }
  });
}

const nav = (page: Page, name: string) => page.getByRole("navigation", { name: "Views" }).getByRole("button", { name }).click();
const screen = (page: Page, name: string) => page.getByRole("region", { name });


/** A cell shows core's number: null is an empty cell (never 0), anything else is that number, grouped or with a % sign. */
async function expectCells(rows: Locator, data: (number | null)[][], firstCol: number) {
  for (const [i, values] of data.entries()) {
    const cells = await rows.nth(i).locator("td").allTextContents();
    const shown = cells.slice(firstCol, firstCol + values.length).map((t) => (t === "" ? null : Number(t.replace(/[,%]/g, ""))));
    expect(shown, `row ${i + 1}`).toEqual(values);
  }
}

/** The numbered fix steps: a terminal command to copy, or a button that starts the work. */
async function expectSteps(list: Locator, actions: Action[]) {
  await expect(list).toHaveCount(actions.length);
  for (const [i, a] of actions.entries()) {
    const li = list.nth(i);
    if (a.kind === "terminal") {
      await expect(li).toContainText("In Terminal");
      await expect(li.locator("code")).toHaveText(a.command);
    } else await expect(li.getByRole("button", { name: a.label })).toBeVisible();
    await expect(li).toContainText(a.does);
  }
}

const glyphOf: Record<StepView["status"], string> = { done: "ok", partial: "warn", skipped: "warn", failed: "bad", waiting: "idle", running: "run" };
/** Each step row shows its glyph, core's word and what came in. */
async function expectStepRows(rows: Locator, steps: StepView[]) {
  await expect(rows).toHaveCount(steps.length);
  for (const [i, s] of steps.entries()) {
    await expect(rows.nth(i).locator(`.g.${glyphOf[s.status]}`)).toBeVisible();
    await expect(rows.nth(i)).toContainText(s.word);
    await expect(rows.nth(i)).toContainText(s.detail);
  }
}

// ---------- the design's eleven states ----------

state("empty", "noworkspace", async (page) => {
  await expect(page.getByRole("heading", { name: "Open a workspace" })).toBeVisible();
  await expect(page.getByRole("button", { name: /Open Workspace/ })).toBeVisible();
  await expect(page.locator(".term code")).toHaveText("moneyball");
});

async function expectToday(page: Page, t: typeof fx.today) {
  const s = screen(page, "Today");
  const day = t.day!;
  await expect(s.getByRole("heading", { name: day.day })).toBeVisible();
  await expect(s.locator(".strip .val")).toHaveText(day.numbers!.map((n) => n.value));
  // a change sits under the number it is about
  const changes = day.numbers!.flatMap((n) => day.versus!.changes.filter((c) => c.label === n.label).map((c) => c.text));
  expect(changes.length).toBe(day.versus!.changes.length);
  await expect(s.locator(".strip .dl")).toHaveText(changes);
  await expectStepRows(s.locator("tbody tr"), t.last_run!.steps);
  await expect(page.locator(".health")).toContainText(t.health_label);
  for (const line of t.footer) await expect(page.locator(".health")).toContainText(line);
  return s;
}

state("today-fresh", "", async (page) => {
  const s = await expectToday(page, fx.today);
  await expect(s.locator(".notice")).toHaveCount(0); // a fresh day has no headline
  await expect(s.locator(".strip .none")).toHaveCount(0);
});

state("today-degraded", "degraded", async (page) => {
  const t = fx.todayDegraded;
  const s = await expectToday(page, t);
  await expect(s.locator(".notice")).toContainText(t.headline!.text);
  await expect(s.locator(".notice")).toContainText(t.headline!.rest);
  // fixes are numbered steps, then what to do next
  await expectSteps(s.locator(".notice li"), [...t.fixes, t.next!]);
  // no CRM is said, never shown as zero
  await expect(s.locator(".strip .none")).toHaveText(t.day!.numbers_note!);
  await expect(s.locator(".strip .val")).toHaveCount(t.day!.numbers!.length);
});

state("brief", "", async (page) => {
  await nav(page, "Brief");
  const s = screen(page, "Brief");
  const b = fx.brief.brief!;
  const rows = s.locator("tbody tr");
  await expect(rows).toHaveCount(b.rows.length);
  await expect(rows.locator("td.nm")).toHaveText(b.rows.map((r) => r.product));
  await expect(s.locator("thead th")).toHaveCount(10);
  await expectCells(rows, b.rows.map((r) => [r.spend_per_day, r.m7d, r.l7d, r.q7d, r.q_per_day, r.rs_per_q, r.l_to_q, r.goal, r.gap]), 1);
  await expect(s.locator(".feas .line")).toHaveText(fx.brief.feasibility_line!);
  await expect(s.locator(".body")).toContainText(fx.brief.cpl_order!);
});

async function expectFunnel(page: Page, v: typeof fx.funnel) {
  await nav(page, "Funnel");
  const s = screen(page, "Funnel");
  const rows = s.locator("tbody tr");
  await expect(rows).toHaveCount(v.rows.length);
  await expect(rows.locator("td.nm")).toHaveText(v.rows.map((r) => r.name));
  await expect(s.locator(".v")).toHaveText(v.rows.map((r) => r.verdict));
  await expectCells(rows, v.rows.map((r) => (v.has_crm ? [r.spend, r.m, r.cpl, r.l, r.q, r.rs_per_q] : [r.spend, r.m, r.cpl])), 1);
  await expect(s.locator(".body")).toContainText(v.cpl_ranking!);
  for (const note of v.notes) await expect(s.locator(".body")).toContainText(note);
  return s;
}

state("funnel", "", async (page) => {
  const s = await expectFunnel(page, fx.funnel);
  await expect(s.locator("thead th")).toHaveCount(8);
  await expect(s.locator(".notice")).toHaveCount(0);
});

state("funnel-nocrm", "degraded", async (page) => {
  const s = await expectFunnel(page, fx.funnelNocrm);
  await expect(s.locator(".notice")).toContainText("Verdicts withheld.");
  await expect(s.locator("thead th")).toHaveText(["Ad set", "Spend", "Meta", "Per lead", "Verdict"]);
  await expect(s.locator(".v.nocrm")).toHaveCount(fx.funnelNocrm.rows.length);
});

const waiting = fx.refreshStateWaiting; // what refresh_state says the moment a run starts
/** Starts a refresh and plays the recorded run up to and including `last` (none when omitted). */
async function startRefresh(page: Page, last?: (ev: RefreshEvent) => boolean) {
  await screen(page, "Today").getByRole("button", { name: "Refresh" }).first().click();
  const upTo = last ? fx.refreshEvents.findIndex(last) + 1 : 0;
  if (last && upTo === 0) throw new Error("the recorded run has no such event");
  for (const ev of fx.refreshEvents.slice(0, upTo)) await emitRefresh(page, ev);
}
const begins = (name: StepView["name"]) => (ev: RefreshEvent) => ev.type === "begin" && ev.step.name === name;
const ends = (name: StepView["name"]) => (ev: RefreshEvent) => ev.type === "done" && ev.step.name === name;
const finished = event(fx.refreshEvents, "finished");

state("refresh-running", "", async (page) => {
  await startRefresh(page, begins("crm")); // meta and assets done, CRM in flight
  const s = screen(page, "Refresh");
  await expect(s.getByRole("heading", { name: `Refreshing ${waiting.day}` })).toBeVisible();
  const rows = s.locator("tbody tr");
  await expect(rows).toHaveCount(waiting.steps.length);
  const at = waiting.steps.findIndex((x) => x.name === "crm");
  const crm = fx.refreshEvents.find(begins("crm")) as Extract<RefreshEvent, { type: "begin" }>;
  await expect(rows.nth(at)).toContainText(crm.step.word);
  await expect(rows.nth(at).locator(".g.run")).toBeVisible();
  await expect(rows.nth(at).locator("td").last()).toHaveText(/^\d\d:\d\d$/); // a ticking clock, not in-step progress
  // the steps no event has reached keep the words refresh_state gave them
  for (const [i, step] of waiting.steps.entries()) if (i > at) await expect(rows.nth(i)).toContainText(step.word);
  await expect(s.getByRole("button", { name: "Cancel" })).toBeVisible();
  // the footer shows the step in flight on every screen
  await expect(page.locator(".health")).toContainText("Refreshing");
  await expect(page.locator(".health")).toContainText(crm.step.label);
});

state("refresh-done", "", async (page) => {
  await startRefresh(page, (ev) => ev.type === "finished");
  const s = screen(page, "Refresh");
  await expect(s.getByRole("heading", { name: `Refreshed ${finished.run.day}` })).toBeVisible();
  await expect(s.locator(".runhead")).toContainText(finished.time);
  await expectStepRows(s.locator("tbody tr"), finished.run.steps);
  for (const n of finished.numbers!) await expect(s.locator(".sum")).toContainText(n.value);
  await expect(s.locator(".sum .notice")).toHaveCount(0); // a clean run has no verdict to give
  await expect(s.getByRole("button", { name: "Refresh Again" })).toBeVisible();
  await expect(s.getByRole("button", { name: "Open Report" })).toBeVisible();
  await expect(page.locator(".health")).toContainText(fx.today.health_label);
});

state("report", "", async (page) => {
  await nav(page, "Report");
  const s = screen(page, "Report");
  const frame = s.locator("iframe");
  await expect(frame).toHaveAttribute("sandbox", "");
  await expect(frame).toHaveAttribute("srcdoc", fx.report.html);
  await expect(s.getByLabel("Report day")).toHaveValue(fx.report.date);
  const values = await s.getByLabel("Report day").locator("option").evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value));
  expect(values).toEqual([...fx.reportDays].sort().reverse());
  await expect(s.getByRole("button", { name: "Open in Browser" })).toBeEnabled();
  const title = /<h1[^>]*>([^<]+)</.exec(fx.report.html)![1]!;
  await expect(page.frameLocator("iframe").getByRole("heading", { name: title })).toBeVisible();
});

const asked = event(fx.turnEvents, "item_done").item as { type: "user"; text: string }; // the recorded turn's question
const answer = event(fx.turnEvents, "assistant_done").text;
/** In a new session, asks the recorded turn's question and plays its events up to and including the first of `last`. */
async function ask(page: Page, last?: (typeof fx.turnEvents)[number]["type"]) {
  await page.keyboard.press("Meta+n");
  const s = screen(page, "Advisor");
  await expect(s.locator(".me")).toHaveCount(0);
  await s.getByLabel("Message").fill(asked.text);
  await s.getByLabel("Message").press("Enter");
  const upTo = last ? fx.turnEvents.findIndex((e) => e.type === last) + 1 : fx.turnEvents.length;
  for (const ev of fx.turnEvents.slice(0, upTo)) await emitTurn(page, ev);
  return s;
}

state("advisor-tool", "", async (page) => {
  const s = await ask(page, "tool_begin");
  const begin = event(fx.turnEvents, "tool_begin");
  await expect(s.locator(".me")).toHaveText([asked.text]);
  const tool = s.locator(".tool").last();
  await expect(tool).toHaveAttribute("aria-busy", "true");
  await expect(tool.locator("code")).toHaveText(begin.name);
  await expect(tool).toContainText("running");
  await expect(s.locator(".stat")).toHaveText(`Running ${begin.name}`);
  await expect(s.getByRole("button", { name: /Stop/ })).toBeVisible();
});

state("advisor-stream", "", async (page) => {
  const s = await ask(page, "assistant_delta"); // the tool is done, the answer has begun
  const delta = event(fx.turnEvents, "assistant_delta").text;
  await expect(s.locator(".tool .g.ok")).toHaveCount(1);
  // markdown is rendered while it streams: the first bold run is bold, its asterisks gone
  const bold = /\*\*(.+?)\*\*/.exec(delta)![1]!;
  await expect(s.locator(".say.live strong").first()).toHaveText(bold);
  await expect(s.locator(".say.live")).not.toContainText("**");
  await expect(s.locator(".stat")).toContainText("Answering");
});

// ---------- states the mock does not draw ----------

state("advisor-done", "", async (page) => {
  const s = await ask(page);
  await expect(s.locator(".say")).toHaveCount(1);
  await expect(s.locator(".say.live")).toHaveCount(0);
  await expect(s.locator(".say strong").first()).toHaveText(/\*\*(.+?)\*\*/.exec(answer)![1]!);
  await expect(s.getByRole("button", { name: "Send" })).toBeVisible();
  await expect(s.locator(".stat")).toHaveText("");
  // the new session had no id until this turn began; the list marks it
  await expect(s.locator(".sess button[aria-current=true] b")).toHaveText(fx.sessions.find((x) => x.id === fx.session.id)!.preview!);
});

state("advisor-session", "", async (page) => {
  // a reopened session: the question, the tool row, the answer with its table
  await nav(page, "Advisor");
  const s = screen(page, "Advisor");
  const items = fx.session.items;
  await expect(s.locator(".me")).toHaveText(items.flatMap((i) => (i.type === "user" ? [i.text] : [])));
  await expect(s.locator(".tool")).toHaveCount(items.filter((i) => i.type === "tool").length);
  await expect(s.locator(".say")).toHaveCount(items.filter((i) => i.type === "assistant").length);
  await expect(s.locator(".sess button")).toHaveCount(fx.sessions.length);
  await expect(s.locator(".sess button b")).toHaveText(fx.sessions.map((x) => x.preview ?? "New session"));
  await expect(s.locator(".sess button").nth(fx.sessions.findIndex((x) => x.id === fx.session.id))).toHaveAttribute("aria-current", "true");
  // a tool row opens to core's output, as it was printed
  await s.locator(".tool").first().click();
  const tool = items.find((i) => i.type === "tool")!;
  await expect(s.locator(".out")).toHaveText(tool.type === "tool" ? tool.output : "");
});

state("advisor-new", "", async (page) => {
  // a new session has no id (the shell writes no file until the first turn): nothing in the list is current
  await page.keyboard.press("Meta+n");
  const s = screen(page, "Advisor");
  expect(fx.sessionNew.id).toBeNull();
  await expect(s.locator(".me, .say, .tool")).toHaveCount(0);
  await expect(s.locator(".log .cap")).toBeVisible();
  await expect(s.locator(".sess button[aria-current=true]")).toHaveCount(0);
  await expect(s.getByRole("button", { name: "Send" })).toBeDisabled();
});

state("advisor-reload", "turnactive", async (page) => {
  await nav(page, "Advisor");
  const s = screen(page, "Advisor");
  await expect(s.locator(".stat")).toContainText("Working on a turn that began before this window reloaded");
  await expect(s.getByRole("button", { name: /Stop/ })).toBeVisible();
});

for (const [flag, kind, typed] of [
  ["nomodel", "no_model", "how far are we from the goal?"],
  ["busy", "busy", "how far are we from the goal?"],
  ["", "not_a_command", "/brief"],
] as const)
  state(`advisor-refused-${kind}`, flag, async (page) => {
    await nav(page, "Advisor");
    const s = screen(page, "Advisor");
    const error = fx.errors[kind]!;
    const before = fx.session.items.filter((i) => i.type === "user").length;
    await s.getByLabel("Message").fill(typed);
    await s.getByLabel("Message").press("Enter");
    await expect(s.locator(".note")).toContainText(error.reason);
    if (error.fix?.kind === "terminal") {
      await expect(s.locator(".note code")).toHaveText(error.fix.command);
      await expect(s.locator(".note")).toContainText(error.fix.does);
    }
    // refused means not sent: the words are back in the composer and the log is unchanged
    await expect(s.getByLabel("Message")).toHaveValue(typed);
    await expect(s.locator(".me")).toHaveCount(before);
    await expect(s.getByRole("button", { name: "Send" })).toBeVisible();
  });

state("advisor-markers", "markers", async (page) => {
  await nav(page, "Advisor");
  const s = screen(page, "Advisor");
  const items = fx.sessionMarkers.items;
  const tools = items.flatMap((i) => (i.type === "tool" ? [i] : []));
  await expect(s.locator(".tool")).toHaveCount(tools.length);
  await expect(s.locator(".tool .g.bad")).toHaveCount(tools.filter((t) => t.status === "failed").length);
  for (const [i, t] of tools.entries()) {
    // a call the interrupt cut short is not shown as still running
    if (t.status === "pending") await expect(s.locator(".tool").nth(i)).toContainText("stopped");
    if (t.status === "failed") await expect(s.locator(".tool").nth(i)).toContainText("failed");
  }
  await expect(s.locator(".tool .g.run")).toHaveCount(0);
  const marks = { turn_interrupted: "Turn stopped", bug_report_filed: "Bug report filed" };
  await expect(s.locator(".mark")).toHaveText(items.flatMap((i) => (i.type in marks ? [marks[i.type as keyof typeof marks]] : [])));
  // a failed call opens to what the tool said
  const failed = tools.findIndex((t) => t.status === "failed");
  await s.locator(".tool").nth(failed).click();
  await expect(s.locator(".out")).toHaveText(tools[failed]!.output);
});

state("advisor-failed", "", async (page) => {
  const s = await ask(page, "tool_begin");
  const save = event(fx.turnEventsOther, "save_failed");
  const fail = event(fx.turnEventsOther, "failed");
  await emitTurn(page, save);
  await emitTurn(page, fail);
  await expect(s.locator(".note")).toHaveText([save.message, fail.error]);
  await expect(s.getByRole("button", { name: "Send" })).toBeVisible();
});

state("advisor-died", "", async (page) => {
  const s = await ask(page, "tool_begin");
  const died = event(fx.turnEventsOther, "died");
  await emitTurn(page, died);
  await expect(s.locator(".note")).toHaveText(died.message);
  await expect(s.locator(".tool .g.run")).toHaveCount(0);
  await expect(s.getByRole("button", { name: "Send" })).toBeVisible();
});

state("today-error", "fail", async (page) => {
  const s = screen(page, "Today");
  const error = fx.errors.snapshot!;
  await expect(s.locator(".notice")).toContainText(error.reason);
  await expectSteps(s.locator(".notice li"), error.fix ? [error.fix] : []);
  await expect(s.getByRole("button", { name: "Refresh" }).first()).toBeVisible(); // the toolbar's, whatever the error offers
  await expect(page.locator(".health")).toContainText("Unavailable");
});

state("refresh-degraded", "degraded", async (page) => {
  const f = fx.refreshFinishedDegraded;
  await startRefresh(page, ends("assets"));
  await emitRefresh(page, f);
  const s = screen(page, "Refresh");
  await expect(s.locator(".sum .notice")).toContainText(f.verdict!.headline);
  await expect(s.locator(".sum .notice")).toContainText(f.verdict!.rest);
  await expectSteps(s.locator(".sum .notice li"), [...f.fixes, ...(f.verdict!.next ? [f.verdict!.next] : []), ...(f.rerun ? [f.rerun] : [])]);
  await expectStepRows(s.locator("tbody tr"), f.run.steps); // "partial" and "skipped" each get the warning glyph and core's word
  await expect(s.locator(".runhead")).toContainText(f.run.gaps);
  await expect(s.locator(".sum")).toContainText(f.numbers_note!);
});

state("refresh-reload", "running", async (page) => {
  // a refresh was already in flight when the window loaded
  await page.locator(".health").click();
  const s = screen(page, "Refresh");
  const st = fx.refreshStateRunning;
  await expect(s.getByRole("heading", { name: `Refreshing ${st.day}` })).toBeVisible();
  const sec = Math.floor(st.started_ms_ago! / 1000);
  const mmss = `${String(Math.floor(sec / 60)).padStart(2, "0")}:${String(sec % 60).padStart(2, "0")}`;
  await expect(s.locator(".runhead span")).toHaveText(`${mmss} so far`);
  await expectStepRows(s.locator("tbody tr"), st.steps);
});

state("refresh-could-not-start", "", async (page) => {
  await startRefresh(page);
  const { error } = event(fx.refreshEventsOther, "could_not_start");
  await emitRefresh(page, { type: "could_not_start", error });
  const s = screen(page, "Refresh");
  await expect(s.locator(".notice")).toContainText(error.reason);
  await expectSteps(s.locator(".notice li"), error.fix ? [error.fix] : []);
  await expect(s.locator(".g.run")).toHaveCount(0);
  await expect(s.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();
});

state("refresh-died", "", async (page) => {
  await startRefresh(page, begins("assets"));
  const died = event(fx.refreshEventsOther, "died");
  await emitRefresh(page, died);
  const s = screen(page, "Refresh");
  await expect(s.locator(".notice")).toContainText(died.message);
  // nothing still spins: the step that was in flight, and those behind it, are bare rings
  await expect(s.locator(".g.run")).toHaveCount(0);
  await expect(s.locator("tbody .g.ok")).toHaveCount(1);
  await expect(s.locator("tbody .g.idle")).toHaveCount(waiting.steps.length - 1);
});

state("workspace-gone", "wsgone", async (page) => {
  const error = fx.errors.no_workspace_config!;
  await expect(page.locator(".notice")).toContainText(error.reason);
  await expectSteps(page.locator(".notice li"), [error.fix!]);
});

state("report-none", "noreport", async (page) => {
  await nav(page, "Report");
  const s = screen(page, "Report");
  await expect(s.locator(".blank")).toContainText("No report is written for this day");
  await expect(s.getByLabel("Report day")).toHaveValue(fx.reportDays[0]!);
  await expect(s.getByRole("button", { name: "Open in Browser" })).toBeDisabled();
  await expect(s.getByRole("button", { name: "Generate" })).toBeEnabled();
  await expect(s.locator("iframe")).toHaveCount(0);
});

// ---------- states only a hand-written fixture reaches (tests/fixtures.ts) ----------

state(
  "funnel-verdicts",
  "",
  async (page) => {
    // every verdict core words, each with its own shape; a real zero beside an unknown
    const s = await expectFunnel(page, funnelAllVerdicts);
    const shape = (v: string) => (v === "keep" ? "keep" : v === "kill" ? "kill" : "wait");
    for (const [i, r] of funnelAllVerdicts.rows.entries()) await expect(s.locator(".v").nth(i)).toHaveClass(`v ${shape(r.verdict)}`);
    await expect(s.locator(".v.wait")).toHaveCount(3);
  },
  { funnel: funnelAllVerdicts },
);

state(
  "today-nodata",
  "",
  async (page) => {
    const s = screen(page, "Today");
    const t = noData.today as typeof fx.today;
    await expect(s.locator(".notice")).toContainText(t.headline!.text);
    await expectSteps(s.locator(".notice li"), [t.next!]);
    await expect(s.locator(".strip")).toHaveCount(0);
    await expect(s.locator("table")).toHaveCount(0);
    await expect(page.locator(".health")).toContainText(t.health_label);
  },
  noData,
);

state(
  "brief-nodata",
  "",
  async (page) => {
    await nav(page, "Brief");
    await expect(screen(page, "Brief").locator(".cap")).toContainText("No snapshot yet");
  },
  noData,
);

state(
  "funnel-nodata",
  "",
  async (page) => {
    await nav(page, "Funnel");
    await expect(screen(page, "Funnel").locator(".cap")).toContainText("Nothing ran for");
    await expect(screen(page, "Funnel").locator("table")).toHaveCount(0);
  },
  noData,
);

state(
  "report-nodata",
  "",
  async (page) => {
    await nav(page, "Report");
    const s = screen(page, "Report");
    await expect(s.locator(".blank")).toContainText("No snapshot yet");
    await expect(s.getByLabel("Report day")).toBeDisabled();
    await expect(s.getByRole("button", { name: "Generate" })).toBeDisabled();
  },
  noData,
);

state(
  "advisor-nodata",
  "",
  async (page) => {
    await nav(page, "Advisor");
    const s = screen(page, "Advisor");
    await expect(s.locator(".sess")).toContainText("No saved sessions yet.");
    await expect(s.locator(".me, .say, .tool")).toHaveCount(0);
  },
  noData,
);

state(
  "refresh-nodata",
  "",
  async (page) => {
    await page.locator(".health").click();
    const s = screen(page, "Refresh");
    await expect(s.locator(".cap")).toContainText("No refresh has run");
    await expect(s.locator("table")).toHaveCount(0);
  },
  noData,
);

state("refresh-crm-failed", "degraded", async (page) => {
  // a failed step says why in a sentence, where the others give counts
  await startRefresh(page, ends("assets"));
  await emitRefresh(page, refreshCrmFailed);
  const s = screen(page, "Refresh");
  await expectStepRows(s.locator("tbody tr"), refreshCrmFailed.run.steps);
  await expect(s.locator("tbody .g.bad")).toHaveCount(1);
  await expectSteps(s.locator(".sum .notice li"), [...refreshCrmFailed.fixes, refreshCrmFailed.rerun!]);
});

state(
  "refresh-reused",
  "",
  async (page) => {
    // a step carried over from today's earlier pull says so, on Refresh and in Today's table
    const reused = refreshReused.run.steps.filter((x) => x.reused).length;
    await expect(screen(page, "Today").locator("tbody .tag")).toHaveCount(reused);
    await startRefresh(page, ends("assets"));
    await emitRefresh(page, refreshReused);
    const s = screen(page, "Refresh");
    await expectStepRows(s.locator("tbody tr"), refreshReused.run.steps);
    await expect(s.locator("tbody .tag")).toHaveText(Array<string>(reused).fill("from today's earlier pull"));
  },
  { today: todayReused },
);
