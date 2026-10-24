// Bookmark (outline) editing (B3-17) in the real app: add, rename, move, delete, undo and redo,
// then save, open the file again and find the same outline.
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

/**
 * A page on the screen, which has the id of the document shown. Any one will do: every page shown
 * has the same id, and a bookmark that goes to another page scrolls page 1 away.
 */
const shownPage = (page: Page) => page.getByRole("main").locator("[data-doc]").first();

/** Does `act`, an edit, and waits until the document shown is the new one: an edit gives it a new id (B2-02). */
async function edit(page: Page, act: () => Promise<void>) {
  const before = await shownPage(page).getAttribute("data-doc");
  await act();
  await expect(shownPage(page)).not.toHaveAttribute("data-doc", before ?? "");
}

/** The titles of the bookmarks shown, in order. */
const titles = (tree: Locator) => tree.getByRole("treeitem").allTextContents();

test("bookmarks edited and saved are the outline of the file opened again (B3-17)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-outline-"));
  try {
    const file = path.join(folder, "test-outline.pdf");
    copyFileSync(corpus("benign/outline-3-levels.pdf"), file);

    let page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const tree = await openOutline(page);
    await expect(tree.getByRole("treeitem", { name: "Chapter 1" })).toBeVisible();

    // Ctrl+B adds a bookmark after Chapter 1, at its level, and starts renaming it.
    await tree.getByRole("treeitem", { name: "Chapter 1" }).click();
    await edit(page, () => page.keyboard.press("Control+b"));
    const input = page.getByLabel(strings.sidebar.renameInputLabel);
    await expect(input).toBeFocused();
    await edit(page, async () => {
      await input.fill("Renamed Bookmark");
      await input.press("Enter");
    });
    const renamed = tree.getByRole("treeitem", { name: "Renamed Bookmark" });
    await expect(renamed).toBeFocused();

    // Alt+↓ puts it after Chapter 2; focus stays on it.
    await edit(page, () => page.keyboard.press("Alt+ArrowDown"));
    await expect(renamed).toBeFocused();
    const moved = await titles(tree);
    expect(moved.indexOf("Renamed Bookmark")).toBe(moved.indexOf("Chapter 2") + 2);

    // Delete takes a bookmark with nothing under it at once; undo and redo.
    await edit(page, () => page.keyboard.press("Delete"));
    await expect(renamed).toHaveCount(0);
    await edit(page, () => page.keyboard.press("Control+z"));
    await expect(renamed).toBeVisible();
    await edit(page, () => page.keyboard.press("Control+y"));
    await expect(renamed).toHaveCount(0);

    // One more, kept, under Chapter 2 (Alt+→ puts it under the item before it).
    await tree.getByRole("treeitem", { name: "Chapter 2" }).click();
    await edit(page, () => page.keyboard.press("Control+b"));
    await edit(page, async () => {
      await input.fill("Persisted New Bookmark");
      await input.press("Enter");
    });
    await edit(page, () => page.keyboard.press("Alt+ArrowRight"));
    const before = await titles(tree);
    expect(before).toContain("Persisted New Bookmark");

    await page.keyboard.press("Control+s");
    await expect(page.getByText(strings.saving.saved)).toBeVisible();
    await quit(page);

    page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");
    const reopened = await openOutline(page);
    await expect(reopened.getByRole("treeitem", { name: "Persisted New Bookmark" })).toBeVisible();
    expect(await titles(reopened)).toEqual(before);
    await quit(page);
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});
