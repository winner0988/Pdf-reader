// Page thumbnails (MVP-18) in the real app: rendered by the worker, and a click goes to the page.
// A sensitive document's thumbnails are page frames, never its pages drawn small (B3-10).
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

test("thumbnails show the pages and take the reader to one", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  // A small window, like the CI runner's: only the first few thumbnails are mounted.
  await page.setViewportSize({ width: 1024, height: 700 });
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

  await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
  const thumbnails = page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
  const first = thumbnails.getByRole("option", { name: strings.canvas.page(1) });
  await expect(first).toHaveAttribute("aria-current", "page");
  // Rendered: the canvas has pixels.
  await expect.poll(() => first.locator("canvas").evaluate((canvas: HTMLCanvasElement) => canvas.width)).toBeGreaterThan(0);

  // Page 7's thumbnail is not mounted yet: the arrow keys scroll to it, as a user would.
  await expect(thumbnails.getByRole("option", { name: strings.canvas.page(7) })).toHaveCount(0);
  await first.focus();
  for (let number = 2; number <= 7; number++) {
    await page.keyboard.press("ArrowDown");
    await expect(thumbnails.getByRole("option", { name: strings.canvas.page(number) })).toBeFocused();
  }
  await page.keyboard.press("Enter");
  const status = page.getByRole("contentinfo");
  await expect(status).toContainText(strings.statusBar.pageStatus(7, 10, strings.toolbar.fitWidth));
  await expect(thumbnails.getByRole("option", { name: strings.canvas.page(7) })).toHaveAttribute("aria-current", "page");
});

test.describe("a sensitive document's thumbnails (B3-10)", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-thumbnails-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("are page frames from the moment it is marked, and still take the reader to a page", async ({ launch }) => {
    // Marking writes to the file: a copy, never the corpus.
    const file = path.join(folder, "pages.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await page.setViewportSize({ width: 1024, height: 700 });
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
    const tab = page.getByRole("tab", { name: strings.sidebar.thumbnailsTab });
    const thumbnails = page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
    const note = page.getByRole("note").filter({ hasText: strings.sidebar.thumbnailsSensitive });
    const status = page.getByRole("contentinfo");

    // Before the mark, the pages are drawn small.
    await tab.click();
    const first = thumbnails.getByRole("option", { name: strings.canvas.page(1) });
    await expect.poll(() => first.locator("canvas").evaluate((canvas: HTMLCanvasElement) => canvas.width)).toBeGreaterThan(0);
    await expect(note).toHaveCount(0);

    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: strings.menu.markSensitive }).click();
    const dialog = page.getByRole("dialog", { name: strings.sensitive.markTitle });
    await expect(dialog).toContainText(strings.sensitive.ready);
    await dialog.getByRole("button", { name: strings.sensitive.mark }).click();
    await expect(status).toContainText(strings.sensitive.badge);

    // What was drawn is gone; every page is still there to go to.
    await tab.click();
    await expect(note).toBeVisible();
    await expect(thumbnails.getByRole("option", { name: strings.canvas.page(1) })).toBeVisible();
    await expect(thumbnails.locator("canvas")).toHaveCount(0);
    await thumbnails.getByRole("option", { name: strings.canvas.page(3) }).click();
    await expect(status).toContainText(strings.statusBar.pageStatus(3, 10, strings.toolbar.fitWidth));

    // Opened again, it is still sensitive: no thumbnail is drawn from the start.
    await page.keyboard.press("Control+w");
    await expect(page.getByRole("heading", { name: strings.empty.title })).toBeVisible();
    await page.keyboard.press("Control+o");
    await answerFileDialog(page, { path: file });
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
    await expect(status).toContainText(strings.sensitive.badge);
    await tab.click();
    await expect(note).toBeVisible();
    await expect(thumbnails.getByRole("option", { name: strings.canvas.page(1) })).toBeVisible();
    await expect(thumbnails.locator("canvas")).toHaveCount(0);
  });
});
