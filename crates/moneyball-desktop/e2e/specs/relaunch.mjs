// Second launch, same home: the advisor session written by the first
// launch is in the list and reopens with everything it held.
import { ANSWER } from "../mock-model.mjs";
import { open, shot } from "./helpers.mjs";

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
});
