// Opening the app against the fixture mock (src/ipc/mock.ts), in manual
// mode: the tests push channel events themselves.
import type { Page } from "@playwright/test";
import type { Mock } from "../src/ipc/mock";
import type { RefreshEvent, TurnEvent } from "../src/ipc/types";

// The fixtures' "now": the morning after the data day, in the workspace's zone.
const NOW = new Date("2026-10-09T10:00:00+05:30");

export async function open(page: Page, flags = "") {
  await page.clock.setFixedTime(NOW);
  await page.goto(`/?mock=manual${flags ? "," + flags : ""}`);
  await page.waitForFunction(() => !!window.__mock);
  // the key handlers exist once the workspace has resolved, either way
  await page.locator(".frame, .empty h2").first().waitFor();
}
export const emitRefresh = (page: Page, ev: RefreshEvent) => page.evaluate((e) => window.__mock!.refresh(e), ev);
export const emitTurn = (page: Page, ev: TurnEvent) => page.evaluate((e) => window.__mock!.turn(e), ev);
export const calls = (page: Page, cmd: string) =>
  page.evaluate((c) => (window.__mock as Mock).calls.filter(([name]) => name === c).map(([, args]) => args), cmd);
