// First launch: every v1 screen through the real IPC and real
// moneyball-core, asserting on what is on screen, one screenshot a step.
import { ANSWER } from "../mock-model.mjs";
import { choose, cmd, modelCalls, nav, open, pixels, screen, shot } from "./helpers.mjs";

const QUESTION = "Which product is cheapest today?";

describe("Moneyball for Mac, first launch", () => {
  it("lands on Today with the remembered workspace", async () => {
    const today = screen("Today");
    await today.waitForDisplayed();
    await today.$(".loading").waitForExist({ reverse: true });
    await expect($(".ws b")).toHaveText("acme-homes");
    // Under the temp home the harness made, so not the person's real one.
    await expect($(".ws span")).toHaveText("~/acme-homes");
    await expect($(".health b")).toHaveText("Stale");
    await expect(nav("Today")).toHaveAttribute("aria-current", "page");
    await shot("01-today");
  });

  it("Cmd-2 shows the Brief table", async () => {
    const brief = await open("Brief", "2");
    await expect(nav("Brief")).toHaveAttribute("aria-current", "page");
    const table = brief.$("table.t");
    await expect(table).toHaveText(expect.stringContaining("Namma Mane"));
    await expect(table).toHaveText(expect.stringContaining("Cityville"));
    // The demo data is weeks old by the real clock: the warning names the button.
    await expect(brief.$(".notice")).toHaveText(expect.stringContaining("Refresh before acting on them."));
    await shot("02-brief");
  });

  it("Funnel changes product, days and level", async () => {
    const funnel = await open("Funnel", "3");
    const rows = () => funnel.$("table.t tbody").getText();
    const head = funnel.$("table.t thead th");
    await expect(head).toHaveText("Ad set");
    const first = await rows();
    await shot("03-funnel");

    await choose(funnel.$('select[aria-label="Product"]'), "Cityville");
    await browser.waitUntil(async () => (await rows()) !== first, { timeoutMsg: "the table did not change with the product" });
    await shot("04-funnel-product");

    await funnel.$("label=28d").click();
    await expect(funnel.$("#seg-days-28d")).toBeChecked();
    await funnel.$("table.t").waitForDisplayed();
    await shot("05-funnel-days");

    await funnel.$("label=Ad").click();
    await expect(head).toHaveText("Ad");
    await shot("06-funnel-level");
  });

  it("Report generates and shows the report with its styles applied", async () => {
    const report = await open("Report", "4");
    await expect(report.$(".blank")).toHaveText(expect.stringContaining("No report is written"));
    await shot("07-report-none");

    await report.$("button=Generate").click();
    const frame = report.$("iframe");
    await frame.waitForDisplayed();
    await expect(report.$("button=Open in Browser")).toBeEnabled();

    // The report's only styling is one inline <style>. Its cards are the
    // one thing on screen painted #fcfbf6 (the frame's own backing is
    // #f4f1e8), so if the app's CSP blocked that style inside the
    // sandboxed frame there would be none of it.
    await browser.waitUntil(async () => (await pixels(frame, "#fcfbf6")) > 20_000, {
      timeoutMsg: "the report rendered without its styles: no card-colored area in the frame",
    });
    await shot("08-report");
  });

  it("Refresh from the footer shows the steps and fails honestly without a Meta token", async () => {
    await $("button.health").click();
    const refresh = screen("Refresh");
    await refresh.waitForDisplayed();
    await shot("09-refresh-before");

    await refresh.$("button=Refresh").click();
    await refresh.$("button=Refresh Again").waitForDisplayed({ timeout: 30_000 });
    const steps = refresh.$$("table.steps tbody tr");
    await expect(steps).toBeElementsArrayOfSize(4);
    // No token: the first step failed, and the screen says how to fix it.
    await expect(steps[0]).toHaveText(expect.stringContaining("failed"));
    await expect(refresh.$(".notice")).toBeDisplayed();
    await expect(refresh.$(".notice")).toHaveText(expect.stringContaining("moneyball"));
    // The fix is worded for an app with no slash commands.
    await expect(refresh.$(".notice")).toHaveText(expect.stringContaining("then open its Meta setup"));
    await expect(refresh.$(".notice")).not.toHaveText(expect.stringContaining("/setup"));
    await expect(refresh.$(".runhead")).toHaveText(expect.stringContaining("stopped at step 1"));
    await shot("10-refresh-failed");
  });

  it("Advisor answers a message with a tool row and then streamed text", async () => {
    const advisor = await open("Advisor", "5");
    // The workspace holds a session the terminal app wrote; it is not this app's to list.
    await expect(advisor.$(".sess")).toHaveText("No saved sessions yet.");
    const before = await modelCalls();

    await advisor.$("textarea").setValue(QUESTION);
    await browser.keys("Enter");
    await expect(advisor.$(".me")).toHaveText(QUESTION);
    const tool = advisor.$("button.tool");
    await tool.waitForDisplayed();
    await expect(tool.$("code")).toHaveText("brief");

    const said = advisor.$(".say:not(.live)");
    await said.waitForDisplayed();
    await advisor.$("button=Send").waitForDisplayed();
    await expect(said).toHaveText(ANSWER.replaceAll("**", ""));
    await expect(said.$("strong")).toHaveText("Keep it running");
    // The tool row holds what the real brief tool returned.
    await tool.click();
    const out = await advisor.$("pre.out");
    await expect(out).toHaveText(expect.stringContaining("Namma Mane"));
    // Long lines wrap: nothing hides behind a sideways scroll.
    expect(await browser.execute((el) => el.scrollWidth <= el.clientWidth, out)).toBe(true);
    expect((await modelCalls()) - before).toBe(2);
    await expect(advisor.$(".sess button b")).toHaveText(QUESTION);
    await shot("11-advisor-answer");
    await tool.click();
  });

  it("Stop ends a slow stream and keeps what had arrived", async () => {
    const advisor = screen("Advisor");
    await advisor.$("textarea").setValue("Answer this one slow, please.");
    await browser.keys("Enter");
    const live = advisor.$(".say.live");
    await live.waitForDisplayed();
    await expect(advisor.$(".stat")).toHaveText(expect.stringContaining("Answering"));
    await shot("12-advisor-streaming");

    await advisor.$("button*=Stop").click();
    await advisor.$("p.mark").waitForDisplayed();
    await expect(advisor.$("p.mark")).toHaveText("Turn stopped");
    await advisor.$("button=Send").waitForDisplayed();
    await expect(live).not.toBeExisting();
    await expect(advisor.$$(".say")[1]).toHaveText(expect.stringContaining("word0"));
    await shot("13-advisor-stopped");
  });

  it("a message starting with / is refused without a model call", async () => {
    const advisor = screen("Advisor");
    const before = await modelCalls();
    await advisor.$("textarea").setValue("/brief");
    await browser.keys("Enter");
    const note = advisor.$(".log .note");
    await note.waitForDisplayed();
    await expect(note).toHaveText(expect.stringMatching(/\S/));
    // Refused: the text is back in the composer and nothing was recorded.
    await expect(advisor.$("textarea")).toHaveValue("/brief");
    await expect(advisor.$$(".me")).toBeElementsArrayOfSize(2);
    expect(await modelCalls()).toBe(before);
    await shot("14-advisor-slash-refused");
  });

  it("New Session starts empty and keeps the old one in the list", async () => {
    const advisor = screen("Advisor");
    await cmd("n");
    await advisor.$(".me").waitForExist({ reverse: true });
    await expect(advisor.$(".log .cap")).toHaveText(expect.stringContaining("Ask about spend"));
    const saved = advisor.$$(".sess button");
    await expect(saved).toBeElementsArrayOfSize(1);
    await expect(saved[0]).not.toHaveAttribute("aria-current");
    await shot("15-advisor-new-session");
  });

  it("the title bar can be dragged and the traffic lights overlap nothing", async () => {
    for (const [label, key] of [["Today", "1"], ["Brief", "2"], ["Funnel", "3"], ["Report", "4"], ["Advisor", "5"]]) {
      await open(label, key);
      const found = await browser.execute(() => {
        // Where the traffic lights sit (window chrome, drawn over the page).
        const W = 80, H = 40;
        const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
        const controls = [...document.querySelectorAll("button, a[href], input, select, textarea, [role=button], [tabindex]")]
          .filter(shown)
          .filter((el) => {
            const r = el.getBoundingClientRect();
            return r.left < W && r.top < H && r.right > 0 && r.bottom > 0;
          })
          .map((el) => el.outerHTML.slice(0, 80));
        // Every point under the lights must belong to a drag region.
        const undraggable = [];
        for (let x = 4; x < W; x += 12)
          for (let y = 4; y < H; y += 12) {
            const el = document.elementFromPoint(x, y);
            if (!el || !el.hasAttribute("data-tauri-drag-region")) undraggable.push(`${x},${y}`);
          }
        const bars = [...document.querySelectorAll("[data-tauri-drag-region]")].filter(shown).length;
        return { controls, undraggable, bars };
      });
      expect({ label, ...found }).toEqual({ label, controls: [], undraggable: [], bars: 2 });
    }
  });
});
