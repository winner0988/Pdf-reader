// The text index (B3-21, ADR 0026) in the real app: off until the user turns it on; the text of the
// files opened then is kept, encrypted, in the app's data folder; once they are closed, the search
// of every document finds them by their words and opens one at the page found; a document marked
// sensitive leaves it at once; clearing deletes it.
import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Locator, Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, dataDir, documentTabs, expect, launchAgain, test } from "./app";

const t = strings.textIndex;
const s = strings.settings;

const indexFolder = (page: Page) => path.join(dataDir(page), "index");

/** The files of the index: the key, the catalog, and one for each document whose text it has. */
const indexFiles = (page: Page) => (existsSync(indexFolder(page)) ? readdirSync(indexFolder(page)) : []);

async function ready(page: Page) {
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
}

async function openSettings(page: Page) {
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: strings.menu.settings }).click();
  return page.getByRole("dialog", { name: s.title });
}

/** Searches every document for `query` in the open dialog, and waits for the search to end. */
async function search(dialog: Locator, query: string) {
  await dialog.getByRole("textbox", { name: t.label }).fill(query);
  await expect(dialog.getByRole("status")).not.toHaveText(t.searching);
  await expect(dialog.getByRole("status")).not.toHaveText("");
}

/** The result of `name` in the list of results. */
const result = (dialog: Locator, name: string) =>
  dialog.getByRole("list", { name: t.results }).getByRole("listitem").filter({ hasText: name });

test.describe("the text index", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-index-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("keeps the text of the files opened, encrypted, finds them by it, and forgets a sensitive one", async ({
    launch,
  }) => {
    // Two files with the same words, in a folder of their own.
    const report = path.join(folder, "report.pdf");
    const notes = path.join(folder, "notes.pdf");
    copyFileSync(corpus("benign/multi-page-10.pdf"), report);
    copyFileSync(corpus("benign/multi-page-10.pdf"), notes);

    // Off at first: a file that opens leaves nothing.
    const page = await launch(report);
    await ready(page);
    expect(indexFiles(page)).toEqual([]);

    const settings = await openSettings(page);
    await settings.getByRole("checkbox", { name: new RegExp(s.textIndex.enable) }).check();
    await page.keyboard.press("Escape");

    // Turned on, the open document goes in, and so does the next one.
    launchAgain(notes);
    await expect(documentTabs(page)).toHaveCount(2);
    await ready(page);
    await expect.poll(() => indexFiles(page).filter((name) => /^d[0-9a-f]{16}$/.test(name)).length, { timeout: 30_000 }).toBe(2);

    // Nothing of the files is on the disk as it is: not their words, names or folder.
    for (const name of indexFiles(page)) {
      const bytes = readFileSync(path.join(indexFolder(page), name));
      for (const plain of ["needle", "keyword", "report.pdf", "notes.pdf", path.basename(folder)]) {
        expect(bytes.includes(Buffer.from(plain, "utf8")), `${name}: ${plain}`).toBe(false);
        expect(bytes.includes(Buffer.from(plain, "utf16le")), `${name}: ${plain}`).toBe(false);
      }
    }

    // Both closed: found by their words from the start screen, with no path on the page.
    await page.keyboard.press("Control+w");
    await expect(documentTabs(page)).toHaveCount(1);
    await page.keyboard.press("Control+w");
    await expect(documentTabs(page)).toHaveCount(0);
    await page.getByRole("button", { name: strings.empty.searchAllButton }).click();
    let dialog = page.getByRole("dialog", { name: t.title });
    await search(dialog, "needle");
    await expect(dialog.getByRole("status")).toHaveText(t.found(2, 2));
    await expect(result(dialog, "report.pdf")).toHaveCount(1);
    await expect(result(dialog, "notes.pdf")).toHaveCount(1);
    expect(await page.content()).not.toContain(path.basename(folder));

    // Opened from there: a new tab, at the page.
    await result(dialog, "report.pdf").getByRole("button", { name: new RegExp(t.page(7)) }).click();
    await expect(dialog).toHaveCount(0);
    await expect(documentTabs(page)).toHaveCount(1);
    await expect(page.getByRole("textbox", { name: strings.toolbar.pageNumber })).toHaveValue("7");

    // Marked as sensitive, a document leaves the index at once: only the other one is found.
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: strings.menu.markSensitive }).click();
    const mark = page.getByRole("dialog", { name: strings.sensitive.markTitle });
    await mark.getByRole("button", { name: strings.sensitive.mark }).click();
    await expect(page.getByRole("contentinfo")).toContainText(strings.sensitive.badge);
    await page.keyboard.press("Control+Shift+F");
    dialog = page.getByRole("dialog", { name: t.title });
    await search(dialog, "needle");
    await expect(dialog.getByRole("status")).toHaveText(t.found(1, 1));
    await expect(result(dialog, "notes.pdf")).toHaveCount(1);
    await expect(result(dialog, "report.pdf")).toHaveCount(0);
    await page.keyboard.press("Escape");

    // Cleared from the settings: nothing of it is left.
    const again = await openSettings(page);
    await again.getByRole("button", { name: s.textIndex.clear }).click();
    await expect(again.getByRole("status").filter({ hasText: s.textIndex.cleared })).toBeVisible();
    expect(existsSync(indexFolder(page))).toBe(false);
  });
});
