// Crash recovery (B2-13) in the real app. The edits of a document the app ended with, unsaved,
// are offered the next time its file opens, in a new run with the same data folder (as after a
// crash or a power cut); saving leaves nothing behind. The file is a copy in a temporary folder.
import { copyFileSync, existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, dataDir, expect, quit, test } from "./app";

const t = strings.recovery;

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) }).first();

/** The toolbar's page count: the document has `total` pages. */
const hasPages = (page: Page, total: number) =>
  expect(page.getByText(strings.toolbar.pageCount(total), { exact: true })).toBeVisible();

/** The recovery journals in the app's data folder `data`. */
function journals(data: string): string[] {
  const folder = path.join(data, "recovery");
  return existsSync(folder) ? readdirSync(folder) : [];
}

test("a page deleted before the app ended is offered back, and nothing is left once saved (B2-13)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-recovery-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await page.getByRole("tab", { name: strings.sidebar.thumbnailsTab }).click();
    const thumbnails = page.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
    await thumbnails.getByRole("option", { name: strings.canvas.page(3) }).click({ button: "right" });
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.pages.delete}`) }).click();
    await hasPages(page, 9);
    const data = dataDir(page);
    expect(journals(data)).toHaveLength(1);
    // The app ends, as in a crash: nothing is saved or discarded on the way out.
    await quit(page);

    const again = await launch(file, { dataDir: data });
    await expect(firstPage(again)).toHaveAttribute("data-state", "ready");
    await hasPages(again, 10);
    const banner = again.getByRole("region", { name: t.label });
    await expect(banner).toContainText(t.available);
    await banner.getByRole("button", { name: t.restore }).click();
    await hasPages(again, 9);
    await expect(banner).toBeHidden();
    await expect(
      again.getByRole("tab", { name: new RegExp(`^original\\.pdf\\s*${strings.tabs.unsaved}$`) }),
    ).toBeVisible();

    await again.keyboard.press("Control+s");
    await expect(again.getByRole("contentinfo")).toContainText(strings.saving.saved);
    expect(journals(data)).toHaveLength(0);
    await quit(again);

    const saved = await launch(file, { dataDir: data });
    await hasPages(saved, 9);
    await expect(firstPage(saved)).toHaveAttribute("data-state", "ready");
    await expect(saved.getByRole("region", { name: t.label })).toHaveCount(0);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
