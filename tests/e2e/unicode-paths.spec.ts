// Files whose name and folder have spaces and Chinese characters, as most of the app's users have,
// through everything that handles a path: the command line, the recent files list, saving over the
// file and saving a copy (the system's dialog). The corpus has no such names, so each test works on
// a copy it makes.
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, dataDir, expect, quit, test } from "./app";

const FOLDER = "我的 PDF 文件";
const NAME = "合約 (最終版) 2026.pdf";

type TauriWindow = {
  __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
};

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) }).first();

/** Whether page 1 is drawn wider than tall: turned a quarter. */
const landscape = (page: Page) =>
  firstPage(page)
    .locator("canvas")
    .evaluate((canvas: HTMLCanvasElement) => canvas.width > canvas.height);

/** Turns page 1 through the app's IPC, in the document whose pages are shown. */
async function rotateFirstPage(page: Page) {
  const doc = Number(await firstPage(page).getAttribute("data-doc"));
  await page.evaluate(async (doc) => {
    await (window as unknown as TauriWindow).__TAURI_INTERNALS__.invoke("apply_edit", {
      args: { doc, edit: { kind: "rotatePages", pages: [0], by: "cw90" } },
    });
  }, doc);
  await expect.poll(() => landscape(page)).toBe(true);
}

test.describe("paths with spaces and Chinese characters", () => {
  let root: string;
  let folder: string;
  let file: string;
  test.beforeEach(() => {
    root = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-unicode-"));
    folder = path.join(root, FOLDER);
    mkdirSync(folder);
    file = path.join(folder, NAME);
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);
  });
  test.afterEach(() => {
    rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("opens from the command line, is listed by name only, and opens again from the list", async ({ launch }) => {
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(page.getByRole("tab", { name: NAME, exact: true })).toBeVisible();
    // The main process wrote the full path, as it is, to the app's data folder.
    const recent = path.join(dataDir(page), "recent.json");
    await expect.poll(() => existsSync(recent)).toBe(true);
    expect(JSON.parse(readFileSync(recent, "utf8")).files).toEqual([file]);

    await page.keyboard.press("Control+w");
    const list = page.getByRole("region", { name: strings.recent.title });
    await expect(list.getByRole("button", { name: NAME, exact: true })).toBeVisible();
    // Nothing on the page says where the file is.
    expect(await page.content()).not.toContain(FOLDER);

    await list.getByRole("button", { name: NAME, exact: true }).click();
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(page.getByRole("tab", { name: NAME, exact: true })).toBeVisible();
  });

  test("Ctrl+S writes an edit into the file under that name", async ({ launch }) => {
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await rotateFirstPage(page);

    await page.keyboard.press("Control+S");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await expect(page.getByRole("tab", { name: NAME, exact: true })).toBeVisible();

    await quit(page);
    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    expect(await landscape(reopened)).toBe(true);
  });

  test("a copy saved from the dialog under a Chinese name is that file", async ({ launch }) => {
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await rotateFirstPage(page);

    await page.keyboard.press("Control+Shift+S");
    const copyName = "副本 (已旋轉) 第二版.pdf";
    const copy = path.join(folder, copyName);
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await expect(page.getByRole("tab", { name: copyName, exact: true })).toBeVisible();
    expect(existsSync(copy)).toBe(true);

    await quit(page);
    const reopened = await launch(copy);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    expect(await landscape(reopened)).toBe(true);
  });
});

test.describe("a name with characters that mean something elsewhere", () => {
  let root: string;
  test.beforeEach(() => {
    root = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-special-"));
  });
  test.afterEach(() => {
    rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  // `%`, `&`, `#`, brackets and a leading dot are fine in a Windows name, and mean something in a URL,
  // a command line or a shell.
  test("opens, is listed, and Ctrl+S writes an edit into the file", async ({ launch }) => {
    const name = "100% 完成 & #1 [最終] {v2} 'a' ＋全形.pdf";
    const file = path.join(root, name);
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);

    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(page.getByRole("tab", { name, exact: true })).toBeVisible();
    await rotateFirstPage(page);
    await page.keyboard.press("Control+S");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);

    await quit(page);
    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    expect(await landscape(reopened)).toBe(true);
  });
});

test.describe("a path longer than 260 characters", () => {
  let root: string;
  test.beforeEach(() => {
    root = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-long-"));
  });
  test.afterEach(() => {
    rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("opens, and Ctrl+S writes an edit into the file", async ({ launch }) => {
    // Seven folders of 40 characters take it well over MAX_PATH, which Windows still imposes on
    // programs that do not say they handle longer paths.
    const folder = path.join(root, ...Array.from({ length: 7 }, (_, index) => `長路徑的資料夾-${index}`.padEnd(40, "x")));
    mkdirSync(folder, { recursive: true });
    const file = path.join(folder, "long.pdf");
    expect(file.length).toBeGreaterThan(300);
    copyFileSync(corpus("benign/multi-page-10.pdf"), file);

    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await rotateFirstPage(page);
    await page.keyboard.press("Control+S");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);

    await quit(page);
    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    expect(await landscape(reopened)).toBe(true);
  });
});

// A Windows account with a Chinese name has the app's data folder under it.
test("an app data folder with Chinese characters and spaces keeps the settings and the recent files", async ({
  launch,
}) => {
  const root = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-unicode-data-"));
  const data = path.join(root, "使用者 王小明", "應用程式 資料");
  mkdirSync(data, { recursive: true });
  const file = corpus("benign/single-page.pdf");
  try {
    const page = await launch(file, { dataDir: data });
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: strings.menu.settings }).click();
    const dialog = page.getByRole("dialog", { name: strings.settings.title });
    await dialog.getByRole("radio", { name: strings.settings.themes.dark }).check();

    const settings = path.join(data, "settings.json");
    await expect.poll(() => (existsSync(settings) ? JSON.parse(readFileSync(settings, "utf8")).theme : null)).toBe("dark");
    expect(JSON.parse(readFileSync(path.join(data, "recent.json"), "utf8")).files).toEqual([file]);

    await quit(page);
    const again = await launch(undefined, { dataDir: data });
    await expect(again.locator("html")).toHaveClass(/dark/);
    const list = again.getByRole("region", { name: strings.recent.title });
    await expect(list.getByRole("button", { name: "single-page.pdf", exact: true })).toBeVisible();
  } finally {
    rmSync(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
