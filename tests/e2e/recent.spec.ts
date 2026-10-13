// The recently opened files (#73) in the real app. The list lives in the test's own data folder
// (tests/e2e/app.ts); the page may show file names only, never where the files are.
import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, dataDir, expect, launchAgain, test } from "./app";

const SINGLE = corpus("benign/single-page.pdf");
const TEN = corpus("benign/multi-page-10.pdf");

const recentJson = (page: Page) => path.join(dataDir(page), "recent.json");
const stored = (page: Page) => readFileSync(recentJson(page), "utf8");

async function ready(page: Page) {
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
}

/** Closes every tab: the start screen shows. */
async function closeAll(page: Page) {
  while ((await page.getByRole("tab").count()) > 0) await page.keyboard.press("Control+w");
  await expect(page.getByRole("heading", { name: strings.empty.title })).toBeVisible();
}

const recentList = (page: Page) => page.getByRole("region", { name: strings.recent.title });

test("an opened file is listed by name on the start screen and opens again from there", async ({ launch }) => {
  const page = await launch(SINGLE);
  await ready(page);
  // The main process wrote the full path to the app's data folder.
  await expect.poll(() => existsSync(recentJson(page))).toBe(true);
  expect(JSON.parse(stored(page)).files).toEqual([SINGLE]);

  await closeAll(page);
  const list = recentList(page);
  await expect(list.getByRole("button", { name: "single-page.pdf", exact: true })).toBeVisible();
  // Nothing on the page says where the file is.
  const html = await page.content();
  expect(html).not.toContain(path.dirname(SINGLE));
  expect(html).not.toContain("corpus");

  await list.getByRole("button", { name: "single-page.pdf", exact: true }).click();
  await ready(page);
  await expect(page.getByRole("tab", { name: /single-page\.pdf/ })).toBeVisible();
});

test("the list is most recent first, and one click clears it", async ({ launch }) => {
  const page = await launch(SINGLE);
  await ready(page);
  launchAgain(TEN);
  await expect(page.getByRole("tab")).toHaveCount(2);
  await expect.poll(() => (existsSync(recentJson(page)) ? JSON.parse(stored(page)).files : [])).toEqual([TEN, SINGLE]);

  await closeAll(page);
  const buttons = recentList(page).getByRole("button", { name: /\.pdf$/ });
  await expect(buttons).toHaveText(["multi-page-10.pdf", "single-page.pdf"]);

  await recentList(page).getByRole("button", { name: strings.recent.clear }).click();
  await expect(recentList(page)).toHaveCount(0);
  // Nothing is left behind.
  expect(existsSync(recentJson(page))).toBe(false);
});

test("「不記錄此檔案」 takes a file off the list and keeps it off, without its path", async ({ launch }) => {
  const page = await launch(SINGLE);
  await ready(page);
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  const item = page.getByRole("menuitemcheckbox", { name: strings.menu.dontRecord });
  await expect(item).toHaveAttribute("aria-checked", "false");
  await item.click();
  await expect(item).toHaveAttribute("aria-checked", "true");
  await page.keyboard.press("Escape");

  const json = stored(page);
  expect(JSON.parse(json).files).toEqual([]);
  expect(JSON.parse(json).excluded).toHaveLength(1);
  expect(json).not.toContain("single-page");

  // Opened again, it is still not recorded.
  await closeAll(page);
  launchAgain(SINGLE);
  await ready(page);
  await closeAll(page);
  await expect(recentList(page)).toHaveCount(0);
  expect(JSON.parse(stored(page)).files).toEqual([]);
});

test("a file that was deleted since says so, and goes off the list", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-gone-"));
  try {
    const file = path.join(folder, "gone.pdf");
    copyFileSync(SINGLE, file);
    const page = await launch(file);
    await ready(page);
    await closeAll(page);
    const button = recentList(page).getByRole("button", { name: "gone.pdf", exact: true });
    await expect(button).toBeVisible();

    rmSync(file);
    await button.click();
    await expect(page.getByRole("alert")).toHaveText(strings.recent.missing("gone.pdf"));
    // No tab was made for it, the list no longer has it, and the file that is stored does not either.
    await expect(page.getByRole("tab")).toHaveCount(0);
    expect(existsSync(recentJson(page)) ? JSON.parse(stored(page)).files : []).toEqual([]);

    // The app is fine: another file opens.
    launchAgain(SINGLE);
    await ready(page);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
