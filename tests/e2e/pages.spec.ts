// Page management (B2-05) in the real app, from the thumbnails. The card's first scenario: page 3
// deleted from the context menu, page 5 moved to the front with "移到…", page 2 turned right; then
// the document is saved as another file and opened again. Which page is where is told by search
// (each page's title is "Page N of 10"; "needle" is on page 7 only) and by shape (a turned page
// is wider than tall). Dragging a thumbnail is checked on its own.
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const pageSlot = (page: Page, number: number) =>
  page.getByRole("img", { name: strings.canvas.page(number) }).first();

async function openThumbnails(page: Page): Promise<Locator> {
  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  return page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
}

const thumb = (list: Locator, number: number) => list.getByRole("option", { name: strings.canvas.page(number) });

/** Waits until the document shown is a new one: an edit gives it a new id (B2-02). */
async function edited(page: Page, before: string | null) {
  await expect(pageSlot(page, 1)).not.toHaveAttribute("data-doc", before ?? "");
}

/** Right-clicks thumbnail `number` and picks `item` from its menu; waits for the edit. */
async function fromMenu(page: Page, list: Locator, number: number, item: string) {
  const doc = await pageSlot(page, 1).getAttribute("data-doc");
  await thumb(list, number).click({ button: "right" });
  await page.getByRole("menuitem", { name: new RegExp(`^${item}`) }).click();
  if (item !== strings.pages.moveTo) await edited(page, doc);
  return doc;
}

/** Searches for `query`: the viewer goes to its first hit. */
async function find(page: Page, query: string) {
  const search = page.getByRole("textbox", { name: strings.search.placeholder });
  if (!(await search.isVisible())) await page.keyboard.press("Control+f");
  await search.fill(query);
  await search.press("Enter");
}

/** The toolbar's page count: the document has `total` pages. */
const hasPages = (page: Page, total: number) =>
  expect(page.getByText(strings.toolbar.pageCount(total), { exact: true })).toBeVisible();

/** The status bar says page `number` of `total` is shown. */
const showsPage = (page: Page, number: number, total: number) =>
  expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(number, total, ""));

test("pages deleted, moved and turned from the thumbnails are so in the file saved (B2-05)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-pages-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const list = await openThumbnails(page);

    await fromMenu(page, list, 3, strings.pages.delete);
    await hasPages(page, 9);
    // Page 5 is now the fourth.
    const doc = await fromMenu(page, list, 4, strings.pages.moveTo);
    const dialog = page.getByRole("dialog", { name: strings.pages.move.title });
    await dialog.getByLabel(strings.pages.move.page).fill("1");
    await dialog.getByRole("button", { name: strings.pages.move.confirm }).click();
    await edited(page, doc);
    // Page 2 is now the third.
    await fromMenu(page, list, 3, strings.pages.rotateCw);
    await showsPage(page, 1, 9);

    await page.keyboard.press("Control+Shift+S");
    const copy = path.join(folder, "rearranged.pdf");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await quit(page);

    const reopened = await launch(copy);
    await expect(pageSlot(reopened, 1)).toHaveAttribute("data-state", "ready");
    // The third page, page 2 as it was, is turned; the second is not.
    const upright = await pageSlot(reopened, 2).boundingBox();
    const turned = await pageSlot(reopened, 3).boundingBox();
    expect(upright && upright.width < upright.height).toBe(true);
    expect(turned && turned.width > turned.height).toBe(true);
    // Nine pages, in the order 5, 1, 2, 4, 6, 7, 8, 9, 10.
    await find(reopened, "needle");
    await showsPage(reopened, 6, 9);
    await find(reopened, "Page 5 of 10");
    await showsPage(reopened, 1, 9);
    await find(reopened, "Page 3 of 10");
    await expect(
      reopened.getByRole("search", { name: strings.search.label }).getByRole("status"),
    ).toHaveText(strings.search.noResults("Page 3 of 10"));
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("dragging a thumbnail moves its page (B2-05)", async ({ launch }) => {
  // Nothing is saved: the corpus file itself is never written.
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
  const list = await openThumbnails(page);
  const doc = await pageSlot(page, 1).getAttribute("data-doc");

  const from = await thumb(list, 2).boundingBox();
  const to = await thumb(list, 1).boundingBox();
  if (!from || !to) throw new Error("the thumbnails are not on screen");
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2, to.y + 10, { steps: 8 });
  await page.mouse.up();
  await edited(page, doc);

  // Page 2 went before page 1.
  await find(page, "Page 1 of 10");
  await showsPage(page, 2, 10);
  await find(page, "Page 2 of 10");
  await showsPage(page, 1, 10);
});

test("three edits undone with Ctrl+Z leave the document as it was, with nothing to save (B2-05)", async ({
  launch,
}) => {
  // Nothing is saved: the corpus file itself is never written.
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
  const list = await openThumbnails(page);
  const unsaved = page.getByRole("tab", { name: new RegExp(`^multi-page-10\\.pdf\\s*${strings.tabs.unsaved}$`) });
  const saved = page.getByRole("tab", { name: "multi-page-10.pdf", exact: true });

  await fromMenu(page, list, 3, strings.pages.delete);
  const doc = await fromMenu(page, list, 4, strings.pages.moveTo);
  const dialog = page.getByRole("dialog", { name: strings.pages.move.title });
  await dialog.getByLabel(strings.pages.move.page).fill("1");
  await dialog.getByRole("button", { name: strings.pages.move.confirm }).click();
  await edited(page, doc);
  await fromMenu(page, list, 3, strings.pages.rotateCw);
  await expect(unsaved).toBeVisible();

  for (let undo = 0; undo < 3; undo++) {
    const before = await pageSlot(page, 1).getAttribute("data-doc");
    await page.keyboard.press("Control+z");
    await edited(page, before);
  }
  // As it was: ten pages in order, none turned, nothing to save.
  await expect(saved).toBeVisible();
  await hasPages(page, 10);
  await find(page, "Page 3 of 10");
  await showsPage(page, 3, 10);
  const second = await pageSlot(page, 2).boundingBox();
  expect(second && second.width < second.height).toBe(true);

  // Ctrl+Y makes the last one undone again: page 3 is deleted once more. (Not from the search
  // field, which keeps its own undo: Esc closes it.)
  await page.keyboard.press("Escape");
  const before = await pageSlot(page, 1).getAttribute("data-doc");
  await page.keyboard.press("Control+y");
  await edited(page, before);
  await expect(unsaved).toBeVisible();
  await hasPages(page, 9);
});

test("undoing an edit of a document opened with a password asks for the password again (#94)", async ({
  launch,
}) => {
  const page = await launch(corpus("benign/encrypted-aes256.pdf"));
  const unlock = page.getByLabel(strings.password.label, { exact: true });
  await unlock.fill("user");
  await unlock.press("Enter");
  await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
  const list = await openThumbnails(page);
  const shape = async () => {
    const box = await pageSlot(page, 1).boundingBox();
    return box && box.width > box.height ? "turned" : "upright";
  };
  expect(await shape()).toBe("upright");
  await fromMenu(page, list, 1, strings.pages.rotateCw);
  expect(await shape()).toBe("turned");

  // The password is not kept: undo asks for it, and says when it is wrong. (While the dialog is
  // up, the page behind it is hidden from the accessibility tree.)
  const t = strings.pages.undoPassword;
  const before = await pageSlot(page, 1).getAttribute("data-doc");
  await page.keyboard.press("Control+z");
  const dialog = page.getByRole("dialog", { name: t.title });
  const field = dialog.getByLabel(t.label);
  await field.fill("wrong");
  await field.press("Enter");
  await expect(dialog.getByRole("alert")).toHaveText(t.wrong);

  await field.fill("user");
  await field.press("Enter");
  await expect(dialog).toBeHidden();
  await edited(page, before);
  expect(await shape()).toBe("upright");
  await expect(page.getByRole("tab", { name: "encrypted-aes256.pdf", exact: true })).toBeVisible();
});
