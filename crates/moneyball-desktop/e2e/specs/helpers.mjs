// Shared by the spec files: where screenshots go, how a screen is found,
// and the questions asked of the stand-in model.
import { fileURLToPath } from "node:url";
import { PNG } from "pngjs";

const SHOTS = fileURLToPath(new URL("../shots/", import.meta.url));

/** Save what the webview shows as shots/<name>.png. */
export const shot = (name) => browser.saveScreenshot(`${SHOTS}${name}.png`);

/** A screen by its name; all six stay mounted, only one is shown. */
export const screen = (label) => $(`section.screen[aria-label="${label}"]`);

/** The sidebar entry for a screen. */
export const nav = (label) => $(`//nav[@aria-label="Views"]/button[starts-with(normalize-space(.), "${label}")]`);

/** Press Cmd-<key>, as the app's own shortcuts expect it. */
export const cmd = (key) => browser.keys(["Meta", key]);

/** Show a screen by its shortcut and wait until it has finished loading. */
export async function open(label, key) {
  await cmd(key);
  const s = screen(label);
  await s.waitForDisplayed();
  await s.$(".loading").waitForExist({ reverse: true });
  return s;
}

/** How many completions the stand-in model has been asked for. */
export async function modelCalls() {
  const res = await fetch(`${process.env.MB_E2E_MOCK}/calls`);
  return (await res.json()).calls;
}

/**
 * Pick an option of a <select> by its text. The menu a real click opens
 * is a native macOS popup outside the page, which WebDriver cannot reach,
 * so the choice is made the way the menu reports it: value, then change.
 */
export async function choose(select, text) {
  await browser.execute(
    (el, text) => {
      el.value = [...el.options].find((o) => o.text === text).value;
      el.dispatchEvent(new Event("change", { bubbles: true }));
    },
    await select,
    text,
  );
}

/**
 * Count the screenshot's pixels of one color inside an element's box.
 * For what WebDriver cannot ask: the report's frame is sandboxed, so its
 * document is out of reach and only the picture says how it rendered.
 */
export async function pixels(el, hex) {
  const png = PNG.sync.read(Buffer.from(await browser.takeScreenshot(), "base64"));
  const box = await browser.execute((e) => {
    const r = e.getBoundingClientRect();
    return { x: r.left, y: r.top, w: r.width, h: r.height, vw: innerWidth };
  }, await el);
  const scale = png.width / box.vw;
  const want = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  let count = 0;
  for (let y = Math.ceil(box.y * scale); y < Math.min(png.height, (box.y + box.h) * scale); y++)
    for (let x = Math.ceil(box.x * scale); x < Math.min(png.width, (box.x + box.w) * scale); x++) {
      const i = (y * png.width + x) * 4;
      // Within 3 of each channel: the snapshot is color-managed.
      if (want.every((c, k) => Math.abs(png.data[i + k] - c) <= 3)) count++;
    }
  return count / (scale * scale);
}
