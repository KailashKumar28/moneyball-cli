// One test per screen state. Each runs at 1200x800 and 900x600, light and
// dark, checks that nothing wraps, clips or overflows, and saves a
// screenshot to shots/ named like the design's (plans/desktop/design/shots).
import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import type { RefreshEvent, TurnEvent } from "../src/ipc/types";
import { emitRefresh, emitTurn, open } from "./mock";

const fixture = <T>(name: string): T =>
  JSON.parse(readFileSync(new URL(`../src/ipc/fixtures/${name}.json`, import.meta.url), "utf8")) as T;
const refreshEvents = fixture<RefreshEvent[]>("refresh_events");
const turnEvents = fixture<TurnEvent[]>("turn_events");
const refreshDegraded = fixture<RefreshEvent>("refresh_finished_degraded");
const [couldNotStart, refreshDied] = fixture<RefreshEvent[]>("refresh_events_other");
const [, turnFailed, , saveFailed] = fixture<TurnEvent[]>("turn_events_other");

const SIZES = [
  [1200, 800],
  [900, 600],
] as const;
const THEMES = ["light", "dark"] as const;
test.use({ timezoneId: "Asia/Kolkata" });

/** The design's own check (shoot.mjs): no label, cell, button or caption wraps, clips or leaves the window. */
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
      if (e.scrollWidth > e.clientWidth + 1 && !e.matches(".sess b,.cap,.sess button")) out.push("clipped " + t);
      if (!e.matches("td,th,.notice li,.inl,.empty .cap") && r.height > lh * 1.9 && ![...e.children].some((c) => getComputedStyle(c).display !== "inline"))
        out.push("wrapped " + t);
      if (e.matches("td,th") && r.height > 32) out.push("tall cell " + t);
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
function state(name: string, flags: string, arrange: (page: Page) => Promise<void>) {
  test(name, async ({ page }) => {
    for (const [w, h] of SIZES)
      for (const theme of THEMES) {
        await page.setViewportSize({ width: w, height: h });
        await page.emulateMedia({ colorScheme: theme });
        await open(page, flags);
        await arrange(page);
        expect(await layoutIssues(page), `${name} ${theme} ${w}`).toEqual([]);
        await page.screenshot({ path: `shots/${name}-${theme}-${w}x${h}.png` });
      }
  });
}

const nav = (page: Page, name: string) => page.getByRole("navigation", { name: "Views" }).getByRole("button", { name }).click();
const screen = (page: Page, name: string) => page.getByRole("region", { name });

// ---------- the design's eleven states ----------

state("empty", "noworkspace", async (page) => {
  await expect(page.getByRole("heading", { name: "Open a workspace" })).toBeVisible();
  await expect(page.getByRole("button", { name: /Open Workspace/ })).toBeVisible();
  await expect(page.locator(".term code")).toHaveText("moneyball");
});

state("today-fresh", "", async (page) => {
  const s = screen(page, "Today");
  await expect(s.getByRole("heading", { name: "Thu 08 Oct" })).toBeVisible();
  await expect(s.locator(".strip .val")).toHaveText(["₹98,412", "47", "44", "9", "₹10,935"]);
  await expect(s.locator(".strip .dl")).toHaveCount(5);
  await expect(s.locator(".notice")).toHaveCount(0);
  await expect(s.locator("tbody tr")).toHaveCount(4);
  await expect(page.locator(".health")).toContainText("Fresh");
});

state("today-degraded", "degraded", async (page) => {
  const s = screen(page, "Today");
  await expect(s.locator(".notice")).toContainText("Thu 08 Oct has no CRM data.");
  // fixes are numbered steps: a terminal command to copy, then a button
  await expect(s.locator(".notice li")).toHaveCount(2);
  await expect(s.locator(".notice li").first()).toContainText("In Terminal");
  await expect(s.locator(".notice li").first().locator("code")).toHaveText("moneyball crm secret leadzump_token");
  await expect(s.locator(".notice li").nth(1).getByRole("button", { name: "Refresh" })).toBeVisible();
  // no CRM is said, never shown as zero
  await expect(s.locator(".strip .val")).toHaveText(["₹96,869", "42"]);
  await expect(s.locator(".strip .none")).toHaveText("No CRM in this report");
  await expect(page.locator(".health")).toContainText("CRM missing");
});

state("brief", "", async (page) => {
  await nav(page, "Brief");
  const s = screen(page, "Brief");
  await expect(s.locator("thead th")).toHaveCount(10);
  await expect(s.locator("tbody tr")).toHaveCount(5);
  // a ratio with no denominator is an empty cell, a real zero stays a zero
  const last = s.locator("tbody tr").last().locator("td");
  await expect(last.nth(1)).toHaveText("0");
  await expect(last.nth(6)).toHaveText("");
  await expect(s.locator(".feas .line")).toContainText("8.0 qualified a day");
});

state("funnel", "", async (page) => {
  await nav(page, "Funnel");
  const s = screen(page, "Funnel");
  await expect(s.locator("tbody tr")).toHaveCount(4);
  await expect(s.locator(".v.keep")).toHaveText("keep");
  await expect(s.locator(".v.kill")).toHaveText("kill");
  await expect(s.locator(".v.wait")).toHaveText(["wait: immature", "wait: low spend"]);
  await expect(s.locator("thead th")).toHaveCount(8);
});

state("funnel-nocrm", "degraded", async (page) => {
  await nav(page, "Funnel");
  const s = screen(page, "Funnel");
  await expect(s.locator(".notice")).toContainText("Verdicts withheld.");
  await expect(s.locator("thead th")).toHaveText(["Ad set", "Spend", "Meta", "Per lead", "Verdict"]);
  await expect(s.locator(".v.nocrm")).toHaveCount(6);
});

async function startRefresh(page: Page, upTo: number) {
  await screen(page, "Today").getByRole("button", { name: "Refresh" }).first().click();
  for (const ev of refreshEvents.slice(0, upTo)) await emitRefresh(page, ev);
}

state("refresh-running", "", async (page) => {
  await startRefresh(page, 5); // meta and assets done, CRM in flight
  const s = screen(page, "Refresh");
  await expect(s.getByRole("heading", { name: "Refreshing Thu 08 Oct" })).toBeVisible();
  const rows = s.locator("tbody tr");
  await expect(rows).toHaveCount(4);
  await expect(rows.nth(2)).toContainText("running");
  await expect(rows.nth(2).locator(".g.run")).toBeVisible();
  await expect(rows.nth(2).locator("td").last()).toHaveText(/^\d\d:\d\d$/); // a ticking clock, not in-step progress
  await expect(rows.nth(3)).toContainText("waiting");
  await expect(s.getByRole("button", { name: "Cancel" })).toBeVisible();
  // the footer shows the step in flight on every screen
  await expect(page.locator(".health")).toContainText("Refreshing");
  await expect(page.locator(".health")).toContainText("CRM");
});

state("refresh-done", "", async (page) => {
  await startRefresh(page, refreshEvents.length);
  const s = screen(page, "Refresh");
  await expect(s.getByRole("heading", { name: "Refreshed Thu 08 Oct" })).toBeVisible();
  await expect(s.locator("tbody .g.ok")).toHaveCount(4);
  await expect(s.locator(".sum")).toContainText("₹98,412");
  await expect(s.getByRole("button", { name: "Refresh Again" })).toBeVisible();
  await expect(s.getByRole("button", { name: "Open Report" })).toBeVisible();
  await expect(page.locator(".health")).toContainText("Fresh");
});

state("report", "", async (page) => {
  await nav(page, "Report");
  const s = screen(page, "Report");
  const frame = s.locator("iframe");
  await expect(frame).toHaveAttribute("sandbox", "");
  await expect(frame).toHaveAttribute("srcdoc", /Fincity Portfolio/);
  await expect(s.getByLabel("Report day")).toHaveValue("2026-10-08");
  await expect(s.getByLabel("Report day").locator("option")).toHaveText(["Thu 08 Oct", "Wed 07 Oct", "Sun 27 Sep", "Tue 11 Aug"]);
  await expect(s.getByRole("button", { name: "Open in Browser" })).toBeEnabled();
  await expect(page.frameLocator("iframe").getByRole("heading", { name: "Fincity Portfolio" })).toBeVisible();
});

async function ask(page: Page, upTo: number) {
  await nav(page, "Advisor");
  const s = screen(page, "Advisor");
  await s.getByLabel("Message").fill("yes, start with Namma Mane. which ad sets?");
  await s.getByLabel("Message").press("Enter");
  for (const ev of turnEvents.slice(0, upTo)) await emitTurn(page, ev);
  return s;
}

state("advisor-tool", "", async (page) => {
  const s = await ask(page, 2); // the funnel tool has begun
  await expect(s.locator(".me")).toHaveCount(2);
  const tool = s.locator(".tool").last();
  await expect(tool).toHaveAttribute("aria-busy", "true");
  await expect(tool).toContainText("funnel");
  await expect(tool).toContainText("running");
  await expect(s.locator(".stat")).toHaveText("Running funnel");
  await expect(s.getByRole("button", { name: /Stop/ })).toBeVisible();
});

state("advisor-stream", "", async (page) => {
  const s = await ask(page, 7); // tool done, two deltas in
  await expect(s.locator(".tool .g.ok")).toHaveCount(2);
  await expect(s.locator(".say.live")).toContainText("still inside the 72 hour");
  await expect(s.locator(".say.live strong").first()).toHaveText("Lookalike");
  await expect(s.locator(".stat")).toContainText("Answering");
  await expect(s.locator(".sess button").first()).toHaveAttribute("aria-current", "true");
});

// ---------- states the mock does not draw ----------

state("advisor-done", "", async (page) => {
  const s = await ask(page, turnEvents.length);
  await expect(s.locator(".say")).toHaveCount(2);
  await expect(s.locator(".say.live")).toHaveCount(0);
  await expect(s.getByRole("button", { name: "Send" })).toBeVisible();
  await expect(s.locator(".stat")).toHaveText("");
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
    await s.getByLabel("Message").fill(typed);
    await s.getByLabel("Message").press("Enter");
    await expect(s.locator(".note")).toBeVisible();
    await expect(s.locator(".note")).toContainText(kind === "no_model" ? "No model is connected" : kind === "busy" ? "still answering" : "no slash commands");
    // refused means not sent: the words are back in the composer and the log is unchanged
    await expect(s.getByLabel("Message")).toHaveValue(typed);
    await expect(s.locator(".me")).toHaveCount(1);
    await expect(s.getByRole("button", { name: "Send" })).toBeVisible();
  });

state("advisor-markers", "markers", async (page) => {
  await nav(page, "Advisor");
  const s = screen(page, "Advisor");
  await expect(s.locator(".tool .g.bad")).toHaveCount(1);
  await expect(s.locator(".tool").first()).toContainText("failed");
  await expect(s.locator(".mark")).toHaveText(["Turn stopped", "Bug report filed"]);
  await s.locator(".tool").first().click();
  await expect(s.locator(".out")).toContainText("did you mean");
});

state("advisor-failed", "", async (page) => {
  const s = await ask(page, 2);
  await emitTurn(page, saveFailed!);
  await emitTurn(page, turnFailed!);
  await expect(s.locator(".note")).toHaveCount(2);
  await expect(s.locator(".note").last()).toContainText("HTTP 529");
  await expect(s.getByRole("button", { name: "Send" })).toBeVisible();
});

state("today-error", "fail", async (page) => {
  const s = screen(page, "Today");
  await expect(s.locator(".notice")).toContainText("could not be read");
  await expect(s.locator(".notice").getByRole("button", { name: "Refresh" })).toBeVisible();
  await expect(page.locator(".health")).toContainText("Unavailable");
});

state("today-nodata", "nodata", async (page) => {
  const s = screen(page, "Today");
  await expect(s.locator(".notice")).toContainText("No data yet.");
  await expect(s.locator(".strip")).toHaveCount(0);
});

state("brief-nodata", "nodata", async (page) => {
  await nav(page, "Brief");
  await expect(screen(page, "Brief").locator(".cap")).toContainText("No snapshot yet");
});

state("report-none", "noreport", async (page) => {
  await nav(page, "Report");
  const s = screen(page, "Report");
  await expect(s.locator(".blank")).toContainText("No report is written for Thu 08 Oct");
  await expect(s.getByRole("button", { name: "Open in Browser" })).toBeDisabled();
  await s.getByRole("button", { name: "Generate" }).click();
  await expect(s.locator("iframe")).toBeVisible();
});

state("refresh-degraded", "degraded", async (page) => {
  await startRefresh(page, 4);
  await emitRefresh(page, refreshDegraded);
  const s = screen(page, "Refresh");
  await expect(s.locator(".sum .notice")).toContainText("Thu 08 Oct has no CRM data.");
  await expect(s.locator(".sum .notice li")).toHaveCount(2);
  await expect(s.locator("tbody .g.bad")).toHaveCount(1);
});

state("refresh-reload", "running", async (page) => {
  // a refresh was already in flight when the window loaded
  await page.locator(".health").click();
  const s = screen(page, "Refresh");
  await expect(s.getByRole("heading", { name: "Refreshing Thu 08 Oct" })).toBeVisible();
  await expect(s.locator(".runhead span")).toHaveText("00:52 so far");
});

state("refresh-could-not-start", "", async (page) => {
  await startRefresh(page, 0);
  await emitRefresh(page, couldNotStart!);
  const s = screen(page, "Refresh");
  await expect(s.locator(".notice")).toContainText("Meta is not connected.");
  await expect(s.locator(".notice code")).toHaveText("moneyball connect meta");
  await expect(s.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();
});

state("refresh-died", "", async (page) => {
  await startRefresh(page, 3);
  await emitRefresh(page, refreshDied!);
  await expect(screen(page, "Refresh").locator(".notice")).toContainText("stopped without finishing");
});

state("workspace-gone", "wsgone", async (page) => {
  await expect(page.locator(".notice")).toContainText("has no Moneyball workspace");
  await expect(page.locator(".notice code")).toHaveText("moneyball");
});
