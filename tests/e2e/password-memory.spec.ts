// Remembering a password (B2-26, ADR 0017) in the real app, with the real Windows Credential
// Manager. Ticked, the password opens the file at the next start without a question, and a notice
// says so and can forget it; not ticked, nothing is kept.
//
// The app keeps these in the Credential Manager of whoever runs the tests, so the tests only touch
// the entry of the one corpus sample they open, and take it away again in `finally`, the way a user
// would (the notice's "forget"). The one test that forgets every password of the account runs only
// on a runner that is thrown away.
import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { dataDir, expect, quit, test } from "./app";
import { asking, leaveNothing, notice, SAMPLE } from "./passwords";

const t = strings.password;
const s = strings.settings.passwords;

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });
const remember = (page: Page) => page.getByRole("checkbox", { name: new RegExp(t.remember) });

/** Types `password` into the form of the tab that asks, ticking "remember" if asked to. */
async function unlock(page: Page, password: string, ticked: boolean) {
  await expect(asking(page)).toBeVisible();
  // Off until the user ticks it.
  await expect(remember(page)).not.toBeChecked();
  if (ticked) await remember(page).check();
  const field = page.getByLabel(t.label, { exact: true });
  await field.fill(password);
  await field.press("Enter");
  await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
}

/** Opens the settings and reads how many passwords the app says it remembers. */
async function count(page: Page): Promise<number> {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.settings}`) }).click();
  const dialog = page.getByRole("dialog", { name: strings.settings.title });
  const section = dialog.getByRole("region", { name: s.title });
  await expect(section).toContainText(new RegExp(`${s.none}|已記住 \\d+ 份`));
  const found = /已記住 (\d+) 份/.exec(await section.innerText());
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  return found ? Number(found[1]) : 0;
}

test("a password the user asks to remember opens the file at the next start without a question, and can be forgotten (B2-26)", async ({
  launch,
}) => {
  test.setTimeout(120_000);
  let page = await launch(SAMPLE);
  const data = dataDir(page);
  try {
    await unlock(page, "user", true);
    // Typed in this time: no notice, but the menu has it to forget.
    await expect(notice(page)).toHaveCount(0);
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await expect(page.getByRole("menuitem", { name: t.forgetMenu })).toBeVisible();
    await page.keyboard.press("Escape");
    await quit(page);

    // The next start: no form, and a notice that says the file opened with its remembered password.
    page = await launch(SAMPLE, { dataDir: data });
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(asking(page)).toHaveCount(0);
    await expect(notice(page)).toContainText(t.recalled.text);
    // It really is open: the sample's words are found.
    await page.keyboard.press("Control+f");
    await page.getByRole("textbox", { name: strings.search.placeholder }).fill("AES-256");
    await expect(page.locator("[data-highlights] polygon[data-current]")).toHaveCount(1);
    // The search bar floats over the notice's buttons: closed, so that they can be pressed.
    await page.keyboard.press("Escape");
    await expect(page.getByRole("search", { name: strings.search.label })).toHaveCount(0);

    // Forgotten from the notice: it goes away, and the status bar says so.
    await notice(page).getByRole("button", { name: t.recalled.forget }).click();
    await expect(notice(page)).toHaveCount(0);
    await expect(page.getByRole("contentinfo")).toContainText(t.forgotten);
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await expect(page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.settings}`) })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: t.forgetMenu })).toHaveCount(0);
    await page.keyboard.press("Escape");
    await quit(page);

    // The start after that asks again.
    page = await launch(SAMPLE, { dataDir: data });
    await expect(asking(page)).toBeVisible();
    await quit(page);
  } finally {
    await leaveNothing(launch, page, data);
  }
});

test("a password typed in without ticking the box is not kept, and a wrong one is not kept even when ticked (B2-26)", async ({
  launch,
}) => {
  test.setTimeout(120_000);
  let page = await launch(SAMPLE);
  const data = dataDir(page);
  try {
    // A wrong password is refused, whatever the box says, and the box stays as the user left it.
    await expect(asking(page)).toBeVisible();
    await remember(page).check();
    const field = page.getByLabel(t.label, { exact: true });
    await field.fill("not the password");
    await field.press("Enter");
    await expect(page.getByRole("alert")).toHaveText(t.wrong);
    await expect(remember(page)).toBeChecked();
    // Un-ticked, the right one opens the file.
    await remember(page).uncheck();
    await field.fill("user");
    await field.press("Enter");
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    const before = await count(page);
    await quit(page);

    // So the next start asks, and the settings count what they did.
    page = await launch(SAMPLE, { dataDir: data });
    await expect(asking(page)).toBeVisible();
    await unlock(page, "owner", false);
    expect(await count(page)).toBe(before);
    await quit(page);
  } finally {
    await leaveNothing(launch, page, data);
  }
});

/**
 * Forgetting all of them removes every password the app remembers for this Windows account, not
 * only this test's: it is done only on a runner that is thrown away, or when asked for.
 */
const MAY_FORGET_ALL = !!process.env.CI || process.env.E2E_FORGET_ALL === "1";

test("the settings count the remembered passwords, and forget all of them after a question (B2-26)", async ({
  launch,
}) => {
  test.skip(
    !MAY_FORGET_ALL,
    "it forgets every remembered password of this Windows account: only on a runner that is thrown away (E2E_FORGET_ALL=1 asks for it)",
  );
  test.setTimeout(120_000);
  let page = await launch(SAMPLE);
  const data = dataDir(page);
  try {
    await unlock(page, "user", true);
    const remembered = await count(page);
    expect(remembered).toBeGreaterThanOrEqual(1);

    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.settings}`) }).click();
    const dialog = page.getByRole("dialog", { name: strings.settings.title });
    await expect(dialog).toContainText(s.count(remembered));
    // The question comes first, and can be turned down.
    await dialog.getByRole("button", { name: s.forgetAll }).click();
    await expect(dialog.getByRole("group", { name: s.confirmTitle })).toContainText(s.confirmText(remembered));
    await dialog.getByRole("button", { name: s.cancel }).click();
    await expect(dialog).toContainText(s.count(remembered));
    await dialog.getByRole("button", { name: s.forgetAll }).click();
    await dialog.getByRole("button", { name: s.confirm }).click();
    await expect(dialog).toContainText(s.forgotAll(remembered));
    await expect(dialog).toContainText(s.none);
    await page.keyboard.press("Escape");

    // The open document no longer has a password to forget, and the next start asks.
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await expect(page.getByRole("menuitem", { name: t.forgetMenu })).toHaveCount(0);
    await page.keyboard.press("Escape");
    await quit(page);
    page = await launch(SAMPLE, { dataDir: data });
    await expect(asking(page)).toBeVisible();
    await quit(page);
  } finally {
    await leaveNothing(launch, page, data);
  }
});
