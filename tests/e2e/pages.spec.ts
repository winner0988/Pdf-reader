// Page management (B2-05) in the real app. Until the thumbnails offer it, the edits are made
// through the app's own IPC, as the page will: page 3 deleted, page 5 moved to the front, page 2
// turned right; then the document is saved as another file and opened again. Which page is where
// is told by search (the word "needle" is on page 7 only) and by shape (a turned page is wider
// than tall).
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

type TauriWindow = {
  __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
};

const pageSlot = (page: Page, number: number) =>
  page.getByRole("img", { name: strings.canvas.page(number) }).first();

/** Applies `edit` through the app's IPC to the document whose pages are shown, and waits for it. */
async function edit(page: Page, edit: Record<string, unknown>) {
  const doc = await pageSlot(page, 1).getAttribute("data-doc");
  await page.evaluate(
    async ([doc, edit]) => {
      await (window as unknown as TauriWindow).__TAURI_INTERNALS__.invoke("apply_edit", {
        args: { doc: Number(doc), edit },
      });
    },
    [doc, edit] as const,
  );
  // An edited document has a new id.
  await expect(pageSlot(page, 1)).not.toHaveAttribute("data-doc", doc ?? "");
}

test("pages deleted, moved and turned are so in the file saved (B2-05)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-pages-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await expect(pageSlot(page, 1)).toHaveAttribute("data-state", "ready");

    // Pages are counted as the document is before each edit.
    await edit(page, { kind: "deletePages", pages: [2] });
    await edit(page, { kind: "movePages", pages: [3], before: 0 });
    await edit(page, { kind: "rotatePages", pages: [2], by: "cw90" });
    await expect(page.getByRole("contentinfo")).toContainText(strings.statusBar.pageStatus(1, 9, ""));

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
    await reopened.keyboard.press("Control+f");
    const search = reopened.getByRole("textbox", { name: strings.search.placeholder });
    const find = async (query: string) => {
      await search.fill(query);
      await search.press("Enter");
    };
    const status = reopened.getByRole("contentinfo");
    // Page 7, the only one with "needle", is the sixth ...
    await find("needle");
    await expect(status).toContainText(strings.statusBar.pageStatus(6, 9, ""));
    // ... page 5 is the first ...
    await find("Page 5 of 10");
    await expect(status).toContainText(strings.statusBar.pageStatus(1, 9, ""));
    // ... and page 3 is gone.
    await find("Page 3 of 10");
    await expect(
      reopened.getByRole("search", { name: strings.search.label }).getByRole("status"),
    ).toHaveText(strings.search.noResults("Page 3 of 10"));
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
