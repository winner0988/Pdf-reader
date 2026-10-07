// Saving some pages of a document as a PDF of their own, and splitting it into files of so many
// pages (B2-06), in the real app. The page only says what; the main process shows the system's
// save or folder dialog (answered here through UI Automation) and the worker writes the files.
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.export;

async function openExport(page: Page) {
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.export}`) }).click();
  return page.getByRole("dialog", { name: t.title });
}

/** What the reader says of the page it shows: "第 1 / 3 頁". */
const pageStatus = (page: Page) => page.getByRole("contentinfo");

test.describe("split", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-split-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("the pages chosen are a PDF of their own, which opens with those pages (B2-06)", async ({ launch }) => {
    const source = path.join(folder, "source.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), source);
    const page = await launch(source);
    const dialog = await openExport(page);
    await dialog.getByRole("radio", { name: t.pdf }).check();
    await dialog.getByRole("textbox", { name: strings.print.pages }).fill("3-5, 7");
    await dialog.getByRole("button", { name: t.start }).click();

    const target = path.join(folder, "some.pdf");
    await answerFileDialog(page, { path: target });
    await expect(pageStatus(page)).toContainText(t.done(4));
    expect(readFileSync(target).subarray(0, 5).toString("latin1")).toBe("%PDF-");
    // The document it came from is as it was: no unsaved changes, and the file is not touched.
    await expect(page.getByRole("tab", { name: new RegExp(`^source\\.pdf\\s*${strings.tabs.unsaved}$`) })).toHaveCount(0);
    await quit(page);

    // The new file has the four pages, in order, and the keyword that was on page 7 of ten.
    const reopened = await launch(target);
    await expect(reopened.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
    await expect(pageStatus(reopened)).toContainText(strings.statusBar.pageStatus(1, 4, strings.toolbar.fitWidth));
    await reopened.keyboard.press("Control+f");
    const input = reopened.getByRole("textbox", { name: strings.search.placeholder });
    await input.fill("Page 3 of 10");
    await input.press("Enter");
    await expect(reopened.locator("[data-highlights] polygon")).not.toHaveCount(0);
    await input.fill("needle");
    await input.press("Enter");
    await expect(reopened.locator("[data-highlights] polygon")).not.toHaveCount(0);
    await input.fill("Page 6 of 10");
    await input.press("Enter");
    await expect(reopened.locator("[data-highlights] polygon")).toHaveCount(0);
  });

  test("so many pages to a file go to the chosen folder, named by their pages (B2-06)", async ({ launch }) => {
    const source = path.join(folder, "report.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), source);
    const out = path.join(folder, "parts");
    mkdirSync(out);
    const page = await launch(source);
    const dialog = await openExport(page);
    const perFile = dialog.getByRole("textbox", { name: t.perFile });
    await perFile.fill("4");
    await expect(dialog.getByRole("radio", { name: t.pdfEvery })).toBeChecked();
    await dialog.getByRole("button", { name: t.start }).click();

    await answerFileDialog(page, { path: out });
    await expect(pageStatus(page)).toContainText(t.splitDone(3, 10));
    expect(readdirSync(out).sort()).toEqual(["report-p1-4.pdf", "report-p5-8.pdf", "report-p9-10.pdf"]);
    expect(existsSync(path.join(out, "report-p9-10.pdf"))).toBe(true);
    await quit(page);

    const last = await launch(path.join(out, "report-p9-10.pdf"));
    await expect(last.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
    await expect(pageStatus(last)).toContainText(strings.statusBar.pageStatus(1, 2, strings.toolbar.fitWidth));
  });

  test("the pages chosen in the thumbnails are offered to be saved as a file (B2-06)", async ({ launch }) => {
    const source = path.join(folder, "source.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), source);
    const page = await launch(source);
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
    await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
    const thumb = (number: number) => page.getByRole("option", { name: strings.canvas.page(number) });
    await thumb(2).click();
    await thumb(4).click({ modifiers: ["Shift"] });
    await thumb(4).click({ button: "right" });
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.pages.saveSelected}`) }).click();

    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog.getByRole("radio", { name: t.pdf })).toBeChecked();
    await expect(dialog.getByRole("textbox", { name: strings.print.pages })).toHaveValue("2-4");
    await dialog.getByRole("button", { name: t.start }).click();
    const target = path.join(folder, "chosen.pdf");
    await answerFileDialog(page, { path: target });
    await expect(pageStatus(page)).toContainText(t.done(3));
    expect(existsSync(target)).toBe(true);
  });
});
