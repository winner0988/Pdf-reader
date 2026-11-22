// Replacing selected pages with the pages of another file (B6-05, #590) in the real app:
// contiguous pages selected in the thumbnails or current page in command palette,
// dialog allows picking page range from source with live preview of page count.
// Supports single-step undo/redo.
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.pages;

const pageSlot = (page: Page, number: number) =>
  page.getByRole("img", { name: strings.canvas.page(number) }).first();

async function openThumbnails(page: Page): Promise<Locator> {
  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  return page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
}

const hasPages = (page: Page, total: number) =>
  expect(page.getByText(strings.toolbar.pageCount(total), { exact: true })).toBeVisible();

async function find(page: Page, query: string) {
  const search = page.getByRole("textbox", { name: strings.search.placeholder });
  if (!(await search.isVisible())) await page.keyboard.press("Control+f");
  await search.fill(query);
  await search.press("Enter");
}

const showsPage = (page: Page, number: number, total: number) =>
  expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(number, total, ""));

test.describe("replace pages with the pages of another file (B6-05, #590)", () => {
  let folder: string;

  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-replace-pages-"));
  });

  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("replace contiguous pages with same number of pages from another file, with undo (#590)", async ({ launch }) => {
    const docPath = path.join(folder, "doc.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), docPath);
    const page = await launch(docPath);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");

    const list = await openThumbnails(page);
    // Select pages 3 and 4
    await list.getByRole("option", { name: strings.canvas.page(3) }).click();
    await list.getByRole("option", { name: strings.canvas.page(4) }).click({ modifiers: ["Shift"] });

    // Open context menu and click "replace pages"
    await list.getByRole("option", { name: strings.canvas.page(4) }).click({ button: "right" });
    await page.getByRole("menuitem", { name: new RegExp(`^${t.replacePages}`) }).click();

    // Select source file
    await answerFileDialog(page, { path: corpus("benign/multi-page-10.pdf") });

    // ReplacePagesDialog is displayed
    const dialog = page.getByRole("dialog", { name: t.replace.title });
    await expect(dialog).toBeVisible();

    // Set range to pages 7 to 8
    const fromInput = dialog.getByLabel(t.replace.from);
    const toInput = dialog.getByLabel(t.replace.to);
    await fromInput.fill("7");
    await toInput.fill("8");

    // Preview shows 10 pages total (10 - 2 + 2 = 10)
    await expect(dialog.getByText(t.replace.previewTitle(10))).toBeVisible();

    // Confirm replacement
    await dialog.getByRole("button", { name: t.replace.confirm }).click();
    await expect(dialog).toHaveCount(0);

    // Verify page count is still 10
    await hasPages(page, 10);

    // Search for "Page 7 of 10": should now be on page 3 as well as page 7
    await find(page, "Page 7 of 10");
    await showsPage(page, 3, 10);

    // Undo: page 3 reverts to "Page 3 of 10"
    await page.keyboard.press("Control+z");
    await find(page, "Page 3 of 10");
    await showsPage(page, 3, 10);

    await quit(page);
  });

  test("replace single page with multiple pages resulting in different page count (#590)", async ({ launch }) => {
    const docPath = path.join(folder, "doc.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), docPath);
    const page = await launch(docPath);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");

    const list = await openThumbnails(page);
    // Select page 2
    await list.getByRole("option", { name: strings.canvas.page(2) }).click();
    await list.getByRole("option", { name: strings.canvas.page(2) }).click({ button: "right" });
    await page.getByRole("menuitem", { name: new RegExp(`^${t.replacePages}`) }).click();

    await answerFileDialog(page, { path: corpus("benign/multi-page-10.pdf") });

    const dialog = page.getByRole("dialog", { name: t.replace.title });
    await expect(dialog).toBeVisible();

    // Set range 1 to 3 (3 source pages replacing 1 page -> 10 - 1 + 3 = 12 pages)
    const fromInput = dialog.getByLabel(t.replace.from);
    const toInput = dialog.getByLabel(t.replace.to);
    await fromInput.fill("1");
    await toInput.fill("3");
    await expect(dialog.getByText(t.replace.previewTitle(12))).toBeVisible();

    await dialog.getByRole("button", { name: t.replace.confirm }).click();
    await expect(dialog).toHaveCount(0);

    // Now has 12 pages
    await hasPages(page, 12);

    await quit(page);
  });

  test("replace current page via command palette with encrypted source file (#590)", async ({ launch }) => {
    const docPath = path.join(folder, "doc.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), docPath);
    const page = await launch(docPath);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");

    // Open command palette
    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: strings.commands.palette });
    const input = palette.getByRole("combobox");
    await input.fill("取代");

    const option = palette.getByRole("option", { name: strings.commands.replaceCurrentPage });
    await expect(option).toHaveAttribute("aria-selected", "true");
    await input.press("Enter");

    // Pick encrypted file
    await answerFileDialog(page, { path: corpus("benign/encrypted-aes256.pdf") });

    // Prompts password dialog
    const pwdDialog = page.getByRole("dialog", { name: t.sourcePassword.title });
    await expect(pwdDialog).toBeVisible();
    const field = pwdDialog.getByLabel(t.sourcePassword.label);
    await field.fill("user");
    await field.press("Enter");
    await expect(pwdDialog).toHaveCount(0);

    // ReplacePagesDialog opens
    const replaceDialog = page.getByRole("dialog", { name: t.replace.title });
    await expect(replaceDialog).toBeVisible();
    await replaceDialog.getByRole("button", { name: t.replace.confirm }).click();
    await expect(replaceDialog).toHaveCount(0);

    await quit(page);
  });
});
