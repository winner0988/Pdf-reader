// The pages of another file put into a document (B2-06) in the real app, from the thumbnails'
// menu: the system's open dialog (answered through UI Automation) names the file, the pages go in
// after the page that was pressed, in their order; undo, redo and saving follow. A file with
// something active in it brings its pages and a word on the banner, and none of the content. An
// encrypted file asks for its password. A crash leaves the edit to say it cannot be restored.
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, dataDir, expect, quit, test } from "./app";

const t = strings.pages;

const pageSlot = (page: Page, number: number) =>
  page.getByRole("img", { name: strings.canvas.page(number) }).first();

async function openThumbnails(page: Page): Promise<Locator> {
  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  return page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
}

/** Right-clicks thumbnail `number` and picks "insert the pages of another file after it". */
async function takePagesAfter(page: Page, list: Locator, number: number) {
  await list.getByRole("option", { name: strings.canvas.page(number) }).click({ button: "right" });
  await page.getByRole("menuitem", { name: new RegExp(`^${t.insertFileAfter}`) }).click();
  // The menu closes with an animation, and keeps the keys pressed meanwhile (#163).
  await expect(page.getByRole("menu")).toHaveCount(0);
}

/** The toolbar's page count: the document has `total` pages. */
const hasPages = (page: Page, total: number) =>
  expect(page.getByText(strings.toolbar.pageCount(total), { exact: true })).toBeVisible();

/** Searches for `query`: the viewer goes to its first hit. */
async function find(page: Page, query: string) {
  const search = page.getByRole("textbox", { name: strings.search.placeholder });
  if (!(await search.isVisible())) await page.keyboard.press("Control+f");
  await search.fill(query);
  await search.press("Enter");
}

/** The status bar says page `number` of `total` is shown. */
const showsPage = (page: Page, number: number, total: number) =>
  expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(number, total, ""));

/** The pages of `mixed-page-sizes.pdf` after page 3 of `multi-page-10.pdf`, as it was made 14 long. */
async function inOrder(page: Page) {
  await find(page, "A4 portrait");
  await showsPage(page, 4, 14);
  await find(page, "A3 landscape");
  await showsPage(page, 6, 14);
  await find(page, "200 x 200 pt");
  await showsPage(page, 7, 14);
  await find(page, "Page 3 of 10");
  await showsPage(page, 3, 14);
  await find(page, "Page 4 of 10");
  await showsPage(page, 8, 14);
  // Out of the search field, whose keys are its own (Ctrl+Z is not the document's there).
  await page.keyboard.press("Escape");
  await expect(page.getByRole("textbox", { name: strings.search.placeholder })).toBeHidden();
}

test("the pages of another file go in after a page, in their order, and are in the saved copy (B2-06)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-merge-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const list = await openThumbnails(page);

    await takePagesAfter(page, list, 3);
    await answerFileDialog(page, { path: corpus("benign/mixed-page-sizes.pdf") });
    await hasPages(page, 14);
    await expect(
      page.getByRole("tab", { name: new RegExp(`^original\\.pdf\\s*${strings.tabs.unsaved}$`) }),
    ).toBeVisible();
    // The pages that came in are selected, and the others are where they were. The list only has
    // the thumbnails in view (it is virtualized, and a small window shows few), so they are counted
    // in what the sidebar says.
    await expect(page.getByText(t.selected(4), { exact: true })).toBeVisible();
    await inOrder(page);

    // Undone and made again (the main process keeps the file for it).
    await page.keyboard.press("Control+z");
    await hasPages(page, 10);
    await page.keyboard.press("Control+y");
    await hasPages(page, 14);

    const copy = path.join(folder, "merged.pdf");
    await page.keyboard.press("Control+Shift+S");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await quit(page);

    const reopened = await launch(copy);
    await expect(pageSlot(reopened, 1)).toHaveAttribute("data-state", "ready");
    await hasPages(reopened, 14);
    await inOrder(reopened);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("a file with something active in it brings its pages and a word on the banner, and none of the content (B2-06)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-merge-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const banner = page.getByRole("region", { name: strings.banner.label });
    await expect(banner).toHaveCount(0);
    const list = await openThumbnails(page);

    await takePagesAfter(page, list, 1);
    await answerFileDialog(page, { path: corpus("malicious/openaction-js.pdf") });
    await hasPages(page, 11);
    // Said, though nothing of it came: the pages are only what they show.
    await expect(banner).toBeVisible();
    // Closed, and it stays closed for what it said: undone and made again, it has no more to say.
    await banner.getByRole("button", { name: strings.banner.dismiss }).click();
    await expect(banner).toHaveCount(0);
    await page.keyboard.press("Control+z");
    await hasPages(page, 10);
    await page.keyboard.press("Control+y");
    await hasPages(page, 11);
    await expect(banner).toHaveCount(0);
    // The pages of another file with another kind of content: that is new to say.
    await takePagesAfter(page, list, 1);
    await answerFileDialog(page, { path: corpus("malicious/launch.pdf") });
    await hasPages(page, 12);
    await expect(banner).toBeVisible();

    const copy = path.join(folder, "merged.pdf");
    await page.keyboard.press("Control+Shift+S");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await expect(banner).toHaveCount(0);
    await quit(page);

    // No script and no open action in the file (its pages say "/OpenAction JavaScript" in words).
    const saved = readFileSync(copy).toString("latin1");
    expect(saved).not.toMatch(/\/S\s*\/JavaScript/);
    expect(saved).not.toMatch(/\/JS\s*[(<]/);
    expect(saved).not.toMatch(/\/OpenAction\s*[<[]/);
    const reopened = await launch(copy);
    await expect(pageSlot(reopened, 1)).toHaveAttribute("data-state", "ready");
    await hasPages(reopened, 12);
    await expect(reopened.getByRole("region", { name: strings.banner.label })).toHaveCount(0);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("an encrypted file asks for its password, and a file its author forbids is refused (B2-06)", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
  const list = await openThumbnails(page);
  const dialog = page.getByRole("dialog", { name: t.sourcePassword.title });
  const field = dialog.getByLabel(t.sourcePassword.label);

  await takePagesAfter(page, list, 2);
  await answerFileDialog(page, { path: corpus("benign/encrypted-aes256.pdf") });
  await field.fill("not the password");
  await field.press("Enter");
  await expect(dialog.getByRole("alert")).toHaveText(t.sourcePassword.wrong);
  await field.fill("user");
  await field.press("Enter");
  await hasPages(page, 11);

  // A file whose author lets the user in but forbids copying: its pages cannot be taken.
  await takePagesAfter(page, list, 2);
  await answerFileDialog(page, { path: corpus("benign/restricted-open-password.pdf") });
  await field.fill("user");
  await field.press("Enter");
  await expect(page.getByRole("alert")).toContainText(t.sourceNotAllowed);
  await hasPages(page, 11);
});

test("a crash after pages were taken from a file says they cannot be restored (B2-06)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-merge-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const list = await openThumbnails(page);
    await takePagesAfter(page, list, 3);
    await answerFileDialog(page, { path: corpus("benign/mixed-page-sizes.pdf") });
    await hasPages(page, 14);
    const data = dataDir(page);
    // The app ends with the pages unsaved, as in a crash: the journal cannot keep the other file.
    await quit(page);

    const again = await launch(file, { dataDir: data });
    await expect(pageSlot(again, 1)).toHaveAttribute("data-state", "ready");
    await hasPages(again, 10);
    const banner = again.getByRole("region", { name: strings.recovery.label });
    await expect(banner).toContainText(strings.recovery.lost);
    await expect(banner.getByRole("button", { name: strings.recovery.restore })).toHaveCount(0);
    await banner.getByRole("button", { name: strings.recovery.discard }).click();
    await expect(banner).toBeHidden();
    await hasPages(again, 10);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
