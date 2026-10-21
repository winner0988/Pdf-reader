// Outline (bookmark) editing (B3-17) in the real app: add, rename, move, delete, undo, redo,
// save to disk, and verify structure upon re-opening.
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, test } from "./app";

const pageSlot = (page: Page, number: number) =>
  page.getByRole("img", { name: strings.canvas.page(number) }).first();

async function openOutline(page: Page): Promise<Locator> {
  await page.getByRole("tab", { name: strings.sidebar.outlineTab }).click();
  return page.getByRole("tree", { name: strings.sidebar.outlineTab });
}

/** Waits until the document shown is a new one: an edit gives it a new id (B2-02). */
async function edited(page: Page, before: string | null) {
  await expect(pageSlot(page, 1)).not.toHaveAttribute("data-doc", before ?? "");
}

test("bookmarks added, renamed, moved, deleted, undone/redone and saved persist upon re-open (B3-17)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-outline-"));
  try {
    const file = path.join(folder, "test-outline.pdf");
    copyFileSync(corpus("benign/outline-3-levels.pdf"), file);

    // 1. Launch app with document
    let page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const tree = await openOutline(page);
    await expect(tree).toBeVisible();

    const initialDoc = await pageSlot(page, 1).getAttribute("data-doc");
    const firstItem = tree.getByRole("treeitem").first();
    await firstItem.click();

    // 2. Add bookmark via Ctrl+B
    await page.keyboard.press("Control+b");
    await edited(page, initialDoc);

    // 3. Rename with F2
    const docAfterAdd = await pageSlot(page, 1).getAttribute("data-doc");
    await page.keyboard.press("F2");
    const input = page.getByLabel(strings.sidebar.renameInputLabel);
    await expect(input).toBeVisible();
    await input.fill("Renamed Bookmark");
    await input.press("Enter");
    await edited(page, docAfterAdd);
    await expect(tree.getByText("Renamed Bookmark")).toBeVisible();

    // 4. Move bookmark down with Alt+ArrowDown
    const docAfterRename = await pageSlot(page, 1).getAttribute("data-doc");
    await tree.getByText("Renamed Bookmark").click();
    await page.keyboard.press("Alt+ArrowDown");
    await edited(page, docAfterRename);

    // 5. Delete bookmark
    const docAfterMove = await pageSlot(page, 1).getAttribute("data-doc");
    await tree.getByText("Renamed Bookmark").click();
    await page.keyboard.press("Delete");
    // If confirmation dialog appears, confirm it
    const confirmBtn = page.getByRole("button", { name: strings.sidebar.deleteConfirmConfirm });
    if (await confirmBtn.isVisible({ timeout: 500 }).catch(() => false)) {
      await confirmBtn.click();
    }
    await edited(page, docAfterMove);
    await expect(tree.getByText("Renamed Bookmark")).toHaveCount(0);

    // 6. Undo the deletion
    const docAfterDelete = await pageSlot(page, 1).getAttribute("data-doc");
    await page.keyboard.press("Control+z");
    await edited(page, docAfterDelete);
    await expect(tree.getByText("Renamed Bookmark")).toBeVisible();

    // 7. Redo the deletion
    const docAfterUndo = await pageSlot(page, 1).getAttribute("data-doc");
    await page.keyboard.press("Control+y");
    await edited(page, docAfterUndo);
    await expect(tree.getByText("Renamed Bookmark")).toHaveCount(0);

    // 8. Add a new bookmark to save
    const docBeforeSaveAdd = await pageSlot(page, 1).getAttribute("data-doc");
    await firstItem.click();
    await page.keyboard.press("Control+b");
    await edited(page, docBeforeSaveAdd);

    const docBeforeSaveRename = await pageSlot(page, 1).getAttribute("data-doc");
    await page.keyboard.press("F2");
    const inputSave = page.getByLabel(strings.sidebar.renameInputLabel);
    await inputSave.fill("Persisted New Bookmark");
    await inputSave.press("Enter");
    await edited(page, docBeforeSaveRename);

    // 9. Save file
    await page.keyboard.press("Control+s");
    await expect(page.getByText(strings.saving.saved)).toBeVisible();

    // 10. Close and re-open to verify persisted outline
    await quit(page);

    page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const reloadedTree = await openOutline(page);
    await expect(reloadedTree.getByText("Persisted New Bookmark")).toBeVisible();
    await quit(page);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
