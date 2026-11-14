// Keys, re-reads and the trust boundary. No screenshots here.
import { expect, test, type Page } from "@playwright/test";
import type { Action } from "../src/ipc/types";
import { event, fx, onboardingDone, reportDaysLateMonth, sessionFailed, todayReportMissing, workspaceChanged } from "./fixtures";
import { calls, emitTurn, open } from "./mock";

test.use({ timezoneId: "Asia/Kolkata", viewport: { width: 1200, height: 800 } });

const current = (page: Page) => page.locator(".navb[aria-current=page]");
const region = (page: Page, name: string) => page.getByRole("region", { name });

test("Cmd-1 to 5 switch screens", async ({ page }) => {
  await open(page);
  for (const [key, name] of [["2", "Brief"], ["3", "Funnel"], ["4", "Report"], ["5", "Advisor"], ["1", "Today"]] as const) {
    await page.keyboard.press(`Meta+${key}`);
    await expect(current(page)).toContainText(name);
    await expect(region(page, name)).toBeVisible();
  }
});

test("Cmd-R starts a refresh and opens it; the finish re-reads today", async ({ page }) => {
  await open(page);
  await expect(region(page, "Today").locator("iframe.doc")).toBeVisible();
  const before = (await calls(page, "today")).length;
  await page.keyboard.press("Meta+r");
  await expect(region(page, "Refresh")).toBeVisible();
  expect(await calls(page, "refresh_start")).toHaveLength(1);
  await region(page, "Refresh").getByRole("button", { name: "Cancel" }).click();
  expect(await calls(page, "refresh_cancel")).toHaveLength(1);
  await page.evaluate(() => window.__mock!.refresh({ type: "died", message: "gone" }));
  await expect.poll(async () => (await calls(page, "today")).length).toBeGreaterThan(before);
});

test("today is re-read when the window regains focus", async ({ page }) => {
  await open(page);
  await expect(region(page, "Today").locator("iframe.doc")).toBeVisible();
  const before = (await calls(page, "today")).length;
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect.poll(async () => (await calls(page, "today")).length).toBe(before + 1);
});

test("Cmd-N opens a new session", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+n");
  await expect(region(page, "Advisor")).toBeVisible();
  await expect.poll(() => calls(page, "session_new")).toHaveLength(1);
  await expect(region(page, "Advisor").locator(".me")).toHaveCount(0);
  await expect(region(page, "Advisor").getByLabel("Message")).toBeFocused();
});

test("Enter sends, Shift-Enter adds a line, Esc stops the turn", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+5");
  const box = region(page, "Advisor").getByLabel("Message");
  await box.fill("one");
  await box.press("Shift+Enter");
  await box.pressSequentially("two");
  await expect(box).toHaveValue("one\ntwo");
  expect(await calls(page, "turn_start")).toHaveLength(0);
  await box.press("Enter");
  expect((await calls(page, "turn_start"))[0]).toMatchObject({ text: "one\ntwo" });
  await expect(box).toHaveValue("");
  // the composer stays typeable while the turn runs
  await box.fill("next");
  await page.keyboard.press("Escape");
  await expect.poll(() => calls(page, "turn_cancel")).toHaveLength(1);
  await expect(region(page, "Advisor").locator(".mark")).toHaveText("Turn stopped");
  await expect(box).toHaveValue("next");
});

test("opening another session swaps the log", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+5");
  const adv = region(page, "Advisor");
  const other = fx.sessions.findIndex((s) => s.id === fx.sessionMarkers.id);
  expect(fx.session.id).not.toBe(fx.sessionMarkers.id);
  await adv.locator(".sess button").nth(other).click();
  expect((await calls(page, "session_open")).at(-1)).toEqual({ id: fx.sessionMarkers.id });
  await expect(adv.locator(".me")).toHaveText(fx.sessionMarkers.items.flatMap((i) => (i.type === "user" ? [i.text] : [])));
  await expect(adv.locator(".sess button").nth(other)).toHaveAttribute("aria-current", "true");
  await expect(adv.locator(".sess button[aria-current=true]")).toHaveCount(1);
});

test("a new session has no id until its first turn starts, then the list marks it", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+n");
  const adv = region(page, "Advisor");
  await expect(adv.locator(".sess button[aria-current=true]")).toHaveCount(0);
  await adv.getByLabel("Message").fill("q");
  await adv.getByLabel("Message").press("Enter");
  // the turn's start wrote the file: the session is marked while the answer is still coming
  const mine = adv.locator(".sess button").nth(fx.sessions.findIndex((s) => s.id === fx.session.id));
  await expect(mine).toHaveAttribute("aria-current", "true");
  await emitTurn(page, event(fx.turnEvents, "turn_complete"));
  await expect(mine).toHaveAttribute("aria-current", "true");
  // and a click on it does not reopen the session already shown
  await adv.locator(".sess button[aria-current=true]").click();
  expect(await calls(page, "session_open")).toHaveLength(0);
});

test("model output is untrusted: no raw HTML, links do not navigate", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+5");
  const adv = region(page, "Advisor");
  await adv.getByLabel("Message").fill("q");
  await adv.getByLabel("Message").press("Enter");
  const text = 'See [the docs](https://example.com/x) <img src=x onerror="window.pwned=1"> <script>window.pwned=1</script> ![pic](https://example.com/p.png)';
  await page.evaluate((t) => {
    window.__mock!.turn({ type: "assistant_done", text: t });
    window.__mock!.turn({ type: "turn_complete", ms: 1, provider: "p", usage: { input: 1, output: 1 } });
  }, text);
  const say = adv.locator(".say").last();
  await expect(say).toContainText("the docs");
  await expect(say.locator("a, img, script, iframe")).toHaveCount(0);
  await say.getByText("the docs").click();
  expect(page.url()).toContain("localhost");
  expect(await page.evaluate(() => (window as { pwned?: number }).pwned)).toBeUndefined();
});

test("Copy sends the command to copy_text and says Copied", async ({ page }) => {
  await open(page, "degraded");
  const copy = region(page, "Today").getByRole("button", { name: "Copy" });
  const fix = fx.todayDegraded.fixes.find((a): a is Extract<Action, { kind: "terminal" }> => a.kind === "terminal")!;
  await copy.click();
  expect(await calls(page, "copy_text")).toEqual([{ text: fix.command }]);
  await expect(region(page, "Today").getByRole("button", { name: "Copied" })).toBeVisible();
});

test("funnel controls ask core for the product, days and level", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+3");
  const f = region(page, "Funnel");
  const [first, second] = fx.workspace.products as [string, string];
  await expect(f.locator("tbody tr")).toHaveCount(fx.funnel.rows.length);
  expect((await calls(page, "funnel")).at(-1)).toEqual({ product: first, days: 7, by: "adset" });
  await expect(f.getByLabel("Product").locator("option")).toHaveText(fx.workspace.products);
  await f.getByLabel("Product").selectOption(second);
  await f.getByText("28d").click();
  await f.getByText("Campaign").click();
  await expect.poll(async () => (await calls(page, "funnel")).at(-1)).toEqual({ product: second, days: 28, by: "campaign" });
  await expect(f.locator("thead th").first()).toHaveText("Campaign");
});

test("report: day and window drive generate; open in browser passes the path", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+4");
  const r = region(page, "Report");
  await expect(r.locator("iframe")).toBeVisible();
  const older = fx.reportDays.find((d) => d.date !== fx.report.date)!.date;
  await r.getByLabel("Report day").selectOption(older);
  await expect.poll(async () => (await calls(page, "report_for")).at(-1)).toEqual({ date: older });
  await r.getByLabel("Window").selectOption({ label: "7 days" });
  await r.getByRole("button", { name: "Generate" }).click();
  expect((await calls(page, "report_generate")).at(-1)).toEqual({ date: older, days: 7 });
  await r.getByRole("button", { name: "Open in Browser" }).click();
  expect((await calls(page, "report_open_in_browser"))[0]).toEqual({ path: fx.report.path });
});

test("Today opens its day's report in the browser, and Details opens the run on Refresh", async ({ page }) => {
  await open(page, "degraded");
  const t = region(page, "Today");
  await t.getByRole("button", { name: "Open in Browser" }).click();
  expect((await calls(page, "report_open_in_browser"))[0]).toEqual({ path: fx.todayDegraded.day!.report!.path });
  await t.locator(".gaps").getByRole("button", { name: "Details" }).click();
  await expect(region(page, "Refresh")).toBeVisible();
});

test("without a report Today offers to generate one and has nothing to open", async ({ page }) => {
  await open(page, "", { today: todayReportMissing });
  const t = region(page, "Today");
  await expect(t.locator("iframe")).toHaveCount(0);
  await expect(t.locator(".missing")).toContainText(todayReportMissing.day!.report_missing!.note);
  await expect(t.getByRole("button", { name: "Open in Browser" })).toBeDisabled();
});

test("Generate builds a report where none is written", async ({ page }) => {
  await open(page, "noreport");
  await page.keyboard.press("Meta+4");
  const r = region(page, "Report");
  await r.getByRole("button", { name: "Generate" }).click();
  await expect(r.locator("iframe")).toBeVisible();
  expect((await calls(page, "report_generate")).at(-1)).toEqual({ date: fx.reportDays[0]!.date, days: 1 });
});

test("report: the month window sends the days the shell gave it, capped where a snapshot ends", async ({ page }) => {
  await open(page, "", { report_days: reportDaysLateMonth });
  await page.keyboard.press("Meta+4");
  const r = region(page, "Report");
  await expect(r.getByLabel("Window").locator("option")).toHaveText(["1 day", "7 days", "Last 28 days"]);
  await r.getByLabel("Window").selectOption({ label: "Last 28 days" });
  await r.getByRole("button", { name: "Generate" }).click();
  expect((await calls(page, "report_generate")).at(-1)).toEqual({ date: fx.report.date, days: 28 });
});

test("a Generate report action builds the data day's daily report, whatever Report had selected, and Today re-reads", async ({ page }) => {
  await open(page, "", { today: todayReportMissing });
  // Report was left on an older day and a 7-day window
  await page.keyboard.press("Meta+4");
  const r = region(page, "Report");
  const older = fx.reportDays.find((d) => d.date !== fx.report.date)!.date;
  await r.getByLabel("Report day").selectOption(older);
  await r.getByLabel("Window").selectOption({ label: "7 days" });
  await page.keyboard.press("Meta+1");
  const before = (await calls(page, "today")).length;
  await region(page, "Today").getByRole("button", { name: "Generate report" }).click();
  await expect(r).toBeVisible();
  // no date (the shell picks the data day) and one day, not the screen's old selections
  expect((await calls(page, "report_generate")).at(-1)).toEqual({ days: 1 });
  await expect(r.locator("iframe")).toBeVisible();
  await expect(r.getByLabel("Report day")).toHaveValue(fx.report.date);
  await expect(r.getByLabel("Window")).toHaveValue("0");
  // the report is on disk now: Today is read again without waiting for a focus
  await expect.poll(async () => (await calls(page, "today")).length).toBeGreaterThan(before);
});

test("a generate on the Report screen keeps its report on screen and re-reads today", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Meta+4");
  const r = region(page, "Report");
  await expect(r.locator("iframe")).toBeVisible();
  const before = (await calls(page, "today")).length;
  await r.getByLabel("Window").selectOption({ label: "7 days" });
  await r.getByRole("button", { name: "Generate" }).click();
  await expect.poll(async () => (await calls(page, "today")).length).toBeGreaterThan(before);
  // the 7-day report it made stays up through the re-read it caused
  await expect(r.locator("iframe")).toHaveAttribute("title", `Report for ${fx.report.day}`);
  await expect(r.getByRole("button", { name: "Open in Browser" })).toBeEnabled();
});

test("the workspace is re-read with today: a product added in the terminal shows after a focus", async ({ page }) => {
  await open(page, "", { workspace_current: fx.workspace });
  await expect(page.locator(".ws b")).toHaveText(fx.workspace.name);
  const before = (await calls(page, "workspace_current")).length;
  await page.evaluate((w) => void (window.__replies!.workspace_current = w), workspaceChanged);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect.poll(async () => (await calls(page, "workspace_current")).length).toBe(before + 1);
  await expect(page.locator(".ws b")).toHaveText(workspaceChanged.name);
  await page.keyboard.press("Meta+3");
  await expect(region(page, "Funnel").getByLabel("Product").locator("option")).toHaveText(workspaceChanged.products);
  // and the refresh state with it: the terminal or the schedule may have run
  expect((await calls(page, "refresh_state")).length).toBeGreaterThan(1);
});

test("a turn that failed while the window was reloading is shown from the snapshot", async ({ page }) => {
  await open(page, "", { session_current: sessionFailed });
  await page.keyboard.press("Meta+5");
  const adv = region(page, "Advisor");
  await expect(adv.locator(".note")).toHaveText(sessionFailed.failed!);
  await expect(adv.getByRole("button", { name: "Send" })).toBeVisible();
});

test("a copy that fails does not say Copied", async ({ page }) => {
  await open(page, "degraded,copyfail");
  const today = region(page, "Today");
  await today.getByRole("button", { name: "Copy" }).first().click();
  await expect.poll(() => calls(page, "copy_text")).toHaveLength(1);
  await expect(today.getByRole("button", { name: "Copied" })).toHaveCount(0);
});

test("the empty state opens a workspace; Cmd-O does the same", async ({ page }) => {
  await open(page, "noworkspace");
  await page.keyboard.press("Meta+o");
  await expect(page.locator(".ws")).toContainText(fx.workspace.name);
  expect(await calls(page, "workspace_pick")).toHaveLength(1);
});

test("the title bar is a drag region and its text is not selectable", async ({ page }) => {
  await open(page);
  const bar = region(page, "Today").locator(".bar");
  await expect(bar).toHaveAttribute("data-tauri-drag-region");
  expect(await bar.evaluate((e) => getComputedStyle(e).webkitUserSelect)).toBe("none");
  expect(await page.locator("td").first().evaluate((e) => getComputedStyle(e).fontVariantNumeric)).toContain("tabular-nums");
});

// ---------- onboarding ----------

test("Create Client sends the ticked accounts with their edited names, then the first refresh runs on screen", async ({ page }) => {
  await open(page, "noworkspace", { onboarding_state: onboardingDone });
  await page.getByRole("button", { name: "Add a Client" }).click();
  const sheet = page.getByRole("dialog");
  await sheet.getByLabel("Client name").fill("Acme Homes");
  const [first, ...rest] = fx.clientAccounts.filter((a) => a.suggested);
  await expect(sheet.getByRole("checkbox", { name: first!.name })).toBeChecked();
  await sheet.getByLabel(`Product name for ${first!.name}`).fill("Namma Mane");
  const [dropped, ...kept] = rest;
  await sheet.getByRole("checkbox", { name: dropped!.name }).uncheck();
  await sheet.getByRole("button", { name: "Create Client" }).click();
  await expect(sheet).toHaveCount(0);
  expect(await calls(page, "client_create")).toEqual([
    {
      name: "Acme Homes",
      folder: null,
      products: [{ account_id: first!.id, name: "Namma Mane" }, ...kept.map((a) => ({ account_id: a.id, name: a.name }))],
    },
  ]);
  // the client opens on its first refresh, unforced, shown live
  await expect(region(page, "Refresh")).toBeVisible();
  await expect.poll(() => calls(page, "refresh_start")).toHaveLength(1);
  expect((await calls(page, "refresh_start"))[0]).toMatchObject({ force: false });
  await expect(page.locator(".ws")).toContainText(fx.workspace.name);
});

test("a folder picked with Change... is where the client goes", async ({ page }) => {
  await open(page, "noworkspace", { onboarding_state: onboardingDone });
  await page.getByRole("button", { name: "Add a Client" }).click();
  const sheet = page.getByRole("dialog");
  await sheet.getByLabel("Client name").fill("Acme Homes");
  await sheet.getByRole("button", { name: "Change..." }).click();
  await expect(sheet.locator(".where")).toContainText(fx.clientFolder.display);
  await expect(sheet.getByRole("checkbox", { name: fx.clientAccounts.find((a) => a.suggested)!.name })).toBeChecked();
  await sheet.getByRole("button", { name: "Create Client" }).click();
  expect((await calls(page, "client_create"))[0]).toMatchObject({ folder: fx.clientFolder.path });
});

test("Cancel and Escape close the sheet and write nothing", async ({ page }) => {
  await open(page, "noworkspace");
  for (const close of ["Cancel", "Escape"]) {
    await page.getByRole("button", { name: "Add a Client" }).click();
    const sheet = page.getByRole("dialog");
    await expect(sheet.getByRole("button", { name: "Continue with Facebook" })).toBeVisible();
    if (close === "Cancel") await sheet.getByRole("button", { name: "Cancel" }).click();
    else await page.keyboard.press("Escape");
    await expect(sheet).toHaveCount(0);
  }
  for (const cmd of ["meta_connect", "meta_connect_cancel", "client_create"]) expect(await calls(page, cmd)).toHaveLength(0);
  await expect(page.getByRole("heading", { name: "Add your first client" })).toBeVisible();
});

test("while the browser is out, Cancel stops the wait and keeps the sheet; Escape stops it and closes", async ({ page }) => {
  await open(page, "noworkspace");
  await page.getByRole("button", { name: "Add a Client" }).click();
  const sheet = page.getByRole("dialog");
  const facebook = sheet.getByRole("button", { name: "Continue with Facebook" });
  await facebook.click();
  await expect(sheet.getByRole("status")).toContainText("Finish in your browser");
  await sheet.getByRole("button", { name: "Cancel" }).click();
  // back where it started, nothing said: the person chose to stop
  await expect(facebook).toBeVisible();
  await expect(sheet.locator(".err")).toHaveCount(0);
  expect(await calls(page, "meta_connect_cancel")).toHaveLength(1);

  await facebook.click();
  await expect(sheet.getByRole("status")).toContainText("Finish in your browser");
  await page.keyboard.press("Escape");
  await expect(sheet).toHaveCount(0);
  await expect.poll(() => calls(page, "meta_connect_cancel")).toHaveLength(2);
  expect(await calls(page, "meta_connect")).toEqual([{}, {}]);
});

test("a token Meta stopped accepting sends the sheet back to Connect Meta, with its reason", async ({ page }) => {
  await open(page, "", { onboarding_state: onboardingDone, client_accounts: { __fail: fx.errors.meta_auth } });
  await page.getByRole("button", { name: "Add a Client..." }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.getByRole("heading", { name: "Connect Meta" })).toBeVisible();
  await expect(sheet).toContainText("Step 1 of 2");
  await expect(sheet.locator(".err")).toHaveText(fx.errors.meta_auth!.reason);
  await expect(sheet.getByRole("button", { name: "Continue with Facebook" })).toBeFocused();
});

test("while the sheet is up the window's shortcuts wait", async ({ page }) => {
  await open(page, "", { onboarding_state: onboardingDone });
  await page.getByRole("button", { name: "Add a Client..." }).click();
  await page.keyboard.press("Meta+r");
  await page.keyboard.press("Meta+2");
  expect(await calls(page, "refresh_start")).toHaveLength(0);
  await expect(page.locator(".navb[aria-current=page]")).toContainText("Today");
});
