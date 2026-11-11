// Second launch, same home: the advisor session written by the first
// launch is in the list and reopens with everything it held, the refresh
// that failed is shown in full from its record, and a change made to the
// workspace outside the app shows when the window is looked at again.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { ANSWER } from "../mock-model.mjs";
import { open, screen, shot } from "./helpers.mjs";

describe("Moneyball for Mac, after a relaunch", () => {
  it("lists the saved session and reopens it", async () => {
    const advisor = await open("Advisor", "5");
    const saved = advisor.$$(".sess button");
    await expect(saved).toBeElementsArrayOfSize(1);
    await expect(saved[0].$("b")).toHaveText("Which product is cheapest today?");
    // A launch starts on a new, empty session.
    await expect(advisor.$(".me")).not.toBeExisting();
    await shot("16-relaunch-list");

    await saved[0].click();
    await advisor.$(".me").waitForDisplayed();
    await expect(saved[0]).toHaveAttribute("aria-current", "true");
    await expect(advisor.$$(".me")).toBeElementsArrayOfSize(2);
    await expect(advisor.$("button.tool code")).toHaveText("brief");
    await expect(advisor.$$(".say")[0]).toHaveText(ANSWER.replaceAll("**", ""));
    await expect(advisor.$("p.mark")).toHaveText("Turn stopped");
    await shot("17-relaunch-reopened");
  });

  it("shows the last refresh in full from its run record", async () => {
    await $("button.health").click();
    const refresh = screen("Refresh");
    await refresh.waitForDisplayed();
    // Nothing ran in this launch: the verdict, the fix and the rerun come from the record.
    await expect(refresh.$(".runhead h2")).toHaveText("Refresh failed");
    await expect(refresh.$(".runhead")).toHaveText(expect.stringContaining("stopped at step 1"));
    await expect(refresh.$$("table.steps tbody tr")).toBeElementsArrayOfSize(4);
    await expect(refresh.$(".notice")).toHaveText(expect.stringContaining("Nothing was written today."));
    await expect(refresh.$(".notice")).toHaveText(expect.stringContaining("then open its Meta setup"));
    await expect(refresh.$(".notice").$("button=Refresh")).toBeDisplayed();
    await expect(refresh.$("button=Refresh Again")).toBeDisplayed();
    await shot("18-relaunch-refresh-recorded");
  });

  it("re-reads the workspace when the window regains focus", async () => {
    const funnel = await open("Funnel", "3");
    const products = () => funnel.$$('select[aria-label="Product"] option').map((o) => o.getText());
    expect(await products()).toEqual(["Namma Mane", "Cityville"]);
    // What setup in the terminal would do: one more product in the workspace's config.
    const path = join(process.env.MB_E2E_WORKSPACE, ".moneyball/config.json");
    const config = JSON.parse(readFileSync(path, "utf8"));
    config.products.push({ name: "Lakeside", ad_account: "act_303" });
    writeFileSync(path, JSON.stringify(config, null, 2));
    await browser.execute(() => window.dispatchEvent(new Event("focus")));
    await browser.waitUntil(async () => (await products()).length === 3, { timeoutMsg: "the new product never showed" });
    expect(await products()).toEqual(["Namma Mane", "Cityville", "Lakeside"]);
    await shot("19-relaunch-workspace-reread");
  });
});
