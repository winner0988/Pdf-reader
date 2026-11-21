// Document watermark end-to-end tests (B6-06, #591).
// Verifies opening the watermark dialog, adding a text watermark on specific pages,
// previewing, applying, undoing, redoing, saving, and verifying watermark persistence.

import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, test } from "./app";

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });
const t = strings.documentWatermark;

test.describe("document watermark (B6-06, #591)", () => {
  let folder: string;

  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-watermark-"));
  });

  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("adds a text watermark to pages 2-3, undos with Ctrl+Z, redos with Ctrl+Y, saves, and reopens", async ({ launch }) => {
    const file = path.join(folder, "sample.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    // 1. Initial search: "草稿" not found
    await page.keyboard.press("Control+f");
    const search = page.getByRole("search", { name: strings.search.label });
    const searchInput = search.getByRole("textbox", { name: strings.search.placeholder });
    const searchStatus = search.getByRole("status");
    await searchInput.fill("草稿");
    await expect(searchStatus).toHaveText(strings.search.noResults("草稿"));
    await page.keyboard.press("Escape");

    // 2. Open "⋯" menu -> "浮水印…"
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(strings.menu.watermark) }).click();

    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toBeVisible();

    // 3. Enter watermark text "草稿"
    const textInput = dialog.getByLabel(strings.batch.watermarkText);
    await textInput.fill("草稿");

    // 4. Select page range "2-3"
    await dialog.getByRole("radio", { name: t.pagesRange }).click();
    const rangeInput = dialog.getByPlaceholder(t.pagesRangePlaceholder);
    await rangeInput.fill("2-3");

    // 5. Apply watermark
    const applyBtn = dialog.getByRole("button", { name: t.apply });
    await expect(applyBtn).toBeEnabled();
    await applyBtn.click();
    await expect(dialog).toBeHidden();
    await expect(page.getByRole("contentinfo")).toContainText(t.applied);

    // 6. Search "草稿": now found on pages 2 and 3 (count 2)
    await page.keyboard.press("Control+f");
    await searchInput.fill("草稿");
    await expect(searchStatus).toHaveText(strings.search.count(1, 2));
    await page.keyboard.press("Escape");

    // 7. Undo with Ctrl+Z: watermark disappears
    await page.keyboard.press("Control+z");
    await page.keyboard.press("Control+f");
    await searchInput.fill("草稿");
    await expect(searchStatus).toHaveText(strings.search.noResults("草稿"));
    await page.keyboard.press("Escape");

    // 8. Redo with Ctrl+Y: watermark reappears
    await page.keyboard.press("Control+y");
    await page.keyboard.press("Control+f");
    await searchInput.fill("草稿");
    await expect(searchStatus).toHaveText(strings.search.count(1, 2));
    await page.keyboard.press("Escape");

    // 9. Save with Ctrl+S
    await page.keyboard.press("Control+s");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);

    // 10. Reopen saved file and verify persistence
    await quit(page);
    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");

    await reopened.keyboard.press("Control+f");
    const rSearch = reopened.getByRole("search", { name: strings.search.label });
    const rSearchInput = rSearch.getByRole("textbox", { name: strings.search.placeholder });
    const rSearchStatus = rSearch.getByRole("status");
    await rSearchInput.fill("草稿");
    await expect(rSearchStatus).toHaveText(strings.search.count(1, 2));
    await quit(reopened);
  });
});
