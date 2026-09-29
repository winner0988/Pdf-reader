// PDF permissions (MVP-19) in the real app. The corpus's restricted samples open without a
// password; what their author forbids is not done, and the app says why. Copied text and
// printing are recorded instead of reaching this computer's clipboard and printers.
import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

type Recorded = { copied: string[]; printed: number };

async function open(launch: (file?: string) => Promise<Page>, file: string) {
  const page = await launch(corpus(file));
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  await page.evaluate(() => {
    const recorded: Recorded = { copied: [], printed: 0 };
    Object.assign(window, { recorded });
    navigator.clipboard.writeText = async (text: string) => {
      recorded.copied.push(text);
    };
    window.print = () => {
      recorded.printed += 1;
    };
  });
  const recorded = () => page.evaluate(() => (window as unknown as { recorded: Recorded }).recorded);
  return { page, recorded };
}

/** Selects the word "Restricted" by double-clicking where search finds it. */
async function selectWord(page: Page) {
  await page.keyboard.press("Control+f");
  const input = page.getByRole("textbox", { name: strings.search.placeholder });
  await input.fill("Restricted");
  await input.press("Enter");
  const hit = page.locator("[data-highlights] polygon:not([data-current])");
  await expect(hit).toHaveCount(1);
  const box = await hit.boundingBox();
  if (!box) throw new Error("the word is not on screen");
  await page.keyboard.press("Escape");
  await page.mouse.dblclick(box.x + box.width / 2, box.y + box.height / 2);
  await expect(page.locator("[data-selection] polygon")).toHaveCount(1);
}

test("a document that forbids copying and printing: text can be selected, not copied or printed", async ({
  launch,
}) => {
  const { page, recorded } = await open(launch, "benign/restricted-no-copy-no-print.pdf");
  const statusBar = page.getByRole("contentinfo");
  await expect(statusBar).toContainText(strings.permissions.restricted("不可複製、不可列印"));

  await selectWord(page);
  await page.keyboard.press("Control+c");
  await expect(statusBar.getByRole("status")).toHaveText(strings.permissions.copyBlocked);

  await page.keyboard.press("Control+p");
  await expect(statusBar.getByRole("status")).toHaveText(strings.permissions.printBlocked);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await expect(page.getByRole("menuitem", { name: new RegExp(strings.menu.print) })).toHaveAttribute(
    "aria-disabled",
    "true",
  );

  expect(await recorded()).toEqual({ copied: [], printed: 0 });
});

test("a document that allows only low-resolution printing prints at 150 dpi, and copies", async ({ launch }) => {
  const { page, recorded } = await open(launch, "benign/restricted-low-res-print.pdf");
  await expect(page.getByRole("contentinfo")).toContainText(strings.permissions.restricted("只能低解析度列印"));

  await selectWord(page);
  await page.keyboard.press("Control+c");
  await expect.poll(async () => (await recorded()).copied).toEqual(["Restricted"]);

  await page.keyboard.press("Control+p");
  const dialog = page.getByRole("dialog", { name: strings.print.title });
  await expect(dialog).toContainText(strings.permissions.lowResNote(150));
  await dialog.getByRole("button", { name: strings.print.next }).click();
  await expect.poll(async () => (await recorded()).printed).toBe(1);
  // A Letter page, 612 points wide, at 150 dpi.
  const width = await page
    .locator("[data-print-pages] img")
    .evaluate((image) => (image as HTMLImageElement).naturalWidth);
  expect(Math.abs(width - (612 * 150) / 72)).toBeLessThanOrEqual(1);
});
