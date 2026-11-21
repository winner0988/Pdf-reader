// Crop pages (B6-04, #589) in the real app, from the thumbnails.
import { copyFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const pageSlot = (page: Page, number: number) =>
  page.getByRole("img", { name: strings.canvas.page(number) }).first();

async function openThumbnails(page: Page): Promise<Locator> {
  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  return page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
}

const thumb = (list: Locator, number: number) => list.getByRole("option", { name: strings.canvas.page(number) });

async function edited(page: Page, before: string | null) {
  await expect(pageSlot(page, 1)).not.toHaveAttribute("data-doc", before ?? "");
}

test("crop odd pages, undo, redo, and save to a new file (B6-04, #589)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-crop-"));
  const file = path.join(folder, "original.pdf");
  copyFileSync(corpus("benign/multi-page-10.pdf"), file);

  const page = await launch(file);
  await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
  const list = await openThumbnails(page);

  // Right-click thumbnail 1 and select Crop Pages
  const docBefore = await pageSlot(page, 1).getAttribute("data-doc");
  await thumb(list, 1).click({ button: "right" });
  await page.getByRole("menuitem", { name: new RegExp(`^${strings.pages.cropPage}`) }).click();

  const dialog = page.getByRole("dialog", { name: strings.pages.crop.title });
  await expect(dialog).toBeVisible();
  // Verify notice is visible
  await expect(dialog.getByRole("note")).toHaveText(strings.pages.crop.notice);

  // Set margins: right 212, bottom 192
  const rightInput = dialog.getByLabel(strings.pages.crop.right);
  await rightInput.fill("212");
  const bottomInput = dialog.getByLabel(strings.pages.crop.bottom);
  await bottomInput.fill("192");

  // Select "odd pages" (奇數頁)
  await dialog.getByLabel(new RegExp(`^${strings.pages.crop.odd(5)}`)).check();

  // Click confirm
  await dialog.getByRole("button", { name: strings.pages.crop.confirm }).click();
  await edited(page, docBefore);

  // Check aspect ratios:
  // Page 1: 612 - 212 = 400 pt, 792 - 192 = 600 pt. Aspect ratio = 400 / 600 ≈ 0.667.
  // Page 2: untouched 612 x 792. Aspect ratio = 612 / 792 ≈ 0.773.
  const box1 = await pageSlot(page, 1).boundingBox();
  expect(box1!.width / box1!.height).toBeCloseTo(400 / 600, 1);

  const box2 = await pageSlot(page, 2).boundingBox();
  expect(box2!.width / box2!.height).toBeCloseTo(612 / 792, 1);

  // Undo (Ctrl+Z): reverts page 1 to ~612/792
  const docAfterCrop = await pageSlot(page, 1).getAttribute("data-doc");
  await page.keyboard.press("Control+z");
  await edited(page, docAfterCrop);
  const box1Undone = await pageSlot(page, 1).boundingBox();
  expect(box1Undone!.width / box1Undone!.height).toBeCloseTo(612 / 792, 1);

  // Redo (Ctrl+Y): back to ~400/600
  const docAfterUndo = await pageSlot(page, 1).getAttribute("data-doc");
  await page.keyboard.press("Control+y");
  await edited(page, docAfterUndo);
  const box1Redone = await pageSlot(page, 1).boundingBox();
  expect(box1Redone!.width / box1Redone!.height).toBeCloseTo(400 / 600, 1);

  // Save As new file
  const copy = path.join(folder, "cropped.pdf");
  await answerFileDialog(page, copy, () => page.keyboard.press("Control+Shift+S"));
  await expect(page).toHaveTitle(new RegExp(path.basename(copy)));

  // Reopen saved file and verify pages 1 and 3 are ~400/600, page 2 is ~612/792
  const reopened = await launch(copy);
  await expect(pageSlot(reopened, 1)).toHaveAttribute("data-state", "ready");

  const rBox1 = await pageSlot(reopened, 1).boundingBox();
  expect(rBox1!.width / rBox1!.height).toBeCloseTo(400 / 600, 1);

  const rBox2 = await pageSlot(reopened, 2).boundingBox();
  expect(rBox2!.width / rBox2!.height).toBeCloseTo(612 / 792, 1);

  const rBox3 = await pageSlot(reopened, 3).boundingBox();
  expect(rBox3!.width / rBox3!.height).toBeCloseTo(400 / 600, 1);
});
