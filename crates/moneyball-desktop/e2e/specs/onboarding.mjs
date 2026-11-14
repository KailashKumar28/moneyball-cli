// Third launch, on a new and empty home: the app onboards a client against
// the stand-in Meta (mock-meta.mjs) and the stand-in broker
// (mock-broker.mjs), in the words of plans/desktop/IPC.md "Onboarding":
// Add a Client, Continue with Facebook (the e2e build follows Meta's
// dialog itself, standing in for the person), the client named and its
// suggested account ticked, Create Client, the first refresh live, then
// the day's report on Today.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { ACCOUNTS, CLIENT, TOKEN, USER } from "../mock-meta.mjs";
import { open, pixels, shot } from "./helpers.mjs";

const SUGGESTED = ACCOUNTS.find((a) => a.name.includes(CLIENT));
const OTHERS = ACCOUNTS.filter((a) => a !== SUGGESTED);

/** What the window shows, as text (hidden screens are not in it). */
const shown = () => $("body").getText();
const sees = (text) => browser.waitUntil(async () => (await shown()).includes(text), { timeoutMsg: `never showed "${text}"` });

/**
 * An input by its label, however the label is attached: aria-label, a
 * wrapping <label>, or <label for>. `checkbox` picks the tick box over
 * the text field when both carry the same name.
 */
function field(name, checkbox = false) {
  const kind = checkbox ? '[@type="checkbox"]' : '[not(@type="checkbox")]';
  const has = `contains(normalize-space(.), "${name}")`;
  return $(
    [`//input${kind}[contains(@aria-label, "${name}")]`, `//label[${has}]//input${kind}`, `//input${kind}[@id = //label[${has}]/@for]`].join(" | "),
  );
}

/** The first of these that is on screen: screens stay mounted, hidden. */
async function visible(selector) {
  let found;
  await browser.waitUntil(
    async () => {
      for (const el of await $$(selector)) if (await el.isDisplayed()) return (found = el);
      return false;
    },
    { timeoutMsg: `nothing on screen matches ${selector}` },
  );
  return found;
}

/** "Fri 13 Nov": the data day as the app names it (yesterday). */
function dataDay() {
  const d = new Date();
  d.setDate(d.getDate() - 1);
  const day = "Sun Mon Tue Wed Thu Fri Sat".split(" ")[d.getDay()];
  const month = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split(" ")[d.getMonth()];
  return `${day} ${String(d.getDate()).padStart(2, "0")} ${month}`;
}

const metaLog = async () => (await fetch(`${process.env.MB_E2E_META}/__log`)).json();
/** How the stand-in's next dialog answers: { error } as a person who declines, { hold_ms } as one still choosing. */
const nextDialog = (answer) => fetch(`${process.env.MB_E2E_META}/__next_dialog?${new URLSearchParams(answer)}`);
/** The broker swapping a code for the token: the one POST Meta sees, and not the app's. */
const isSwap = (line) => /^POST \/v[\d.]+\/oauth\/access_token$/.test(line);
const AUTH = () => join(process.env.MB_E2E_HOME, ".moneyball/auth.json");

describe("Moneyball for Mac, a new Mac onboarding its first client", () => {
  it("first launch offers to add the first client", async () => {
    const heading = $('//*[self::h1 or self::h2 or self::h3][normalize-space() = "Add your first client"]');
    await heading.waitForDisplayed({ timeout: 20_000 });
    await expect($("button=Add a Client")).toBeDisplayed();
    await expect($("button*=Open a Folder...")).toBeDisplayed();
    // No terminal command any more: the app is the way in.
    expect(await shown()).not.toContain("In Terminal");
    await shot("20-onboarding-first-launch");
  });

  it("Continue with Facebook: Cancel and a decline store nothing, then it connects", async () => {
    await $("button=Add a Client").click();
    await sees("Step 1 of 2");
    // A function: the button is a new element each time the page comes back to it.
    const facebook = () => $("button=Continue with Facebook");
    await facebook().waitForDisplayed();
    // Nothing to paste: no field on the page at all.
    expect(await $$("dialog input")).toHaveLength(0);
    await shot("21-onboarding-connect");

    // Still choosing on Facebook: the app waits, and Cancel stops it. The
    // dialog's late answer then finds no one listening, so nothing lands.
    await nextDialog({ hold_ms: 2000 });
    await facebook().click();
    await sees("Finish in your browser");
    await shot("22-onboarding-connect-waiting");
    await $("button=Cancel").click();
    await facebook().waitForDisplayed();
    await expect($("dialog")).toBeDisplayed();
    await browser.waitUntil(async () => (await metaLog()).some(isSwap), { timeoutMsg: "the held dialog never came back" });
    await browser.pause(500); // the broker's page posting to the closed listener
    expect(existsSync(AUTH())).toBe(false);

    // The person declines on Facebook: the broker's words, nothing stored.
    await nextDialog({ error: "access_denied" });
    await facebook().click();
    await sees("Facebook sign-in was cancelled, so nothing was connected.");
    await expect($("button=Next")).not.toBeDisplayed();
    expect(existsSync(AUTH())).toBe(false);
    await shot("23-onboarding-connect-declined");

    await facebook().click();
    // Who the token is (Meta's /me) and how long it lasts (debug_token).
    await sees(USER.name);
    await sees("Never expires");
    await expect($("button=Next")).toBeDisplayed();
    // Stored where the terminal reads it too; the page never had it.
    expect(JSON.parse(readFileSync(AUTH(), "utf8")).meta_access_token).toBe(TOKEN);
    expect(await shown()).not.toContain(TOKEN);
    // Three trips through Meta's dialog; two codes swapped (the cancelled one's and this one).
    const log = await metaLog();
    expect(log.filter((l) => l.endsWith("/dialog/oauth"))).toHaveLength(3);
    expect(log.filter(isSwap)).toHaveLength(2);
    await shot("24-onboarding-connected");
  });

  it("Add a Client ticks the account whose name holds the client's", async () => {
    await $("button=Next").click();
    await sees("Step 2 of 2");
    await field("Client name").setValue(CLIENT);
    // Where it will live, under the new home.
    await sees(`~/Moneyball/${CLIENT}`);
    await expect($("button=Change...")).toBeDisplayed();
    // Every page of the token's accounts, each with Meta's status in a word.
    for (const a of ACCOUNTS) await field(a.name, true).waitForExist();
    await sees("disabled");
    await browser.waitUntil(() => field(SUGGESTED.name, true).isSelected(), { timeoutMsg: `${SUGGESTED.name} never arrived ticked` });
    for (const a of OTHERS) await expect(field(a.name, true)).not.toBeSelected();
    await expect($("button=Back")).toBeDisplayed();
    await expect($("button=Cancel")).toBeDisplayed();
    await shot("25-onboarding-add-client");
  });

  it("Create Client starts the first refresh, live, and Meta lands with CRM not connected", async () => {
    await $("button=Create Client").click();
    // The stand-in holds the Meta pull for 3 s: the steps show while it runs.
    const steps = await visible("table.steps");
    const rows = steps.$$("tbody tr");
    await expect(rows).toBeElementsArrayOfSize(4);
    await expect(rows[0]).not.toHaveText(expect.stringContaining("done"));
    await shot("26-onboarding-refresh-live");

    await browser.waitUntil(async () => (await rows[3].getText()).includes("no CRM"), { timeout: 30_000, timeoutMsg: "the first refresh never wrote its report" });
    await expect(rows[0]).toHaveText(expect.stringContaining("done"));
    await expect(rows[0]).toHaveText(expect.stringContaining("1 product"));
    await expect(rows[1]).toHaveText(expect.stringContaining("done"));
    await expect(rows[2]).toHaveText(expect.stringContaining("skipped"));
    await expect(rows[2]).toHaveText(expect.stringMatching(/not connected|no CRM connected/));
    await shot("27-onboarding-refresh-done");

    // The client on disk: its folder, the one ticked account as its product.
    const config = JSON.parse(readFileSync(join(process.env.MB_E2E_HOME, "Moneyball", CLIENT, ".moneyball/config.json"), "utf8"));
    expect(config.products.map((p) => p.ad_account)).toEqual([SUGGESTED.id.replace("act_", "")]);
    // Read only (the broker's code swaps aside), and only the ticked account was pulled.
    const log = await metaLog();
    expect(log.filter((l) => !l.startsWith("GET ") && !isSwap(l))).toEqual([]);
    for (const a of OTHERS) expect(log).not.toContain(`GET /${a.id}/insights`);
  });

  it("Today shows the day's report", async () => {
    const today = await open("Today", "1");
    await expect($(".ws b")).toHaveText(CLIENT);
    const doc = today.$("iframe.doc");
    await doc.waitForDisplayed();
    await expect(doc).toHaveAttribute("title", `Report for ${dataDay()}`);
    // Styled, as in the smoke spec: the masthead's ink is a stylesheet color.
    await browser.waitUntil(async () => (await pixels(doc, "#17140f")) > 500, {
      timeoutMsg: "Today's report rendered without its styles",
    });
    await expect(today.$("button=Open in Browser")).toBeEnabled();
    await shot("28-onboarding-today-report");
  });
});
