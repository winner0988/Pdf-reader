// Saving (B2-02) in the real app. There is no editing UI yet (page management, B2-05, adds it),
// so the edit is made through the app's own IPC, as the page would: page 1 is turned a quarter,
// in the document whose pages are shown (`data-doc`).
// The system's save dialog is answered through UI Automation; the window is closed as its close
// button does.
import { createHash } from "node:crypto";
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, appRunning, closeAppWindow, corpus, expect, quit, test } from "./app";

type TauriWindow = {
  __TAURI_INTERNALS__: { invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown> };
};

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) }).first();

/** Whether page 1 is drawn wider than tall: turned a quarter. */
const landscape = (page: Page) =>
  firstPage(page)
    .locator("canvas")
    .evaluate((canvas: HTMLCanvasElement) => canvas.width > canvas.height);

/** The tab of the edited document, marked as having unsaved changes. */
const unsavedTab = (page: Page) =>
  page.getByRole("tab", { name: new RegExp(`^original\\.pdf\\s*${strings.tabs.unsaved}$`) });

const hash = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

/** Opens a copy of a corpus file from a folder of its own. */
async function openCopy(launch: (file?: string) => Promise<Page>, folder: string) {
  const file = path.join(folder, "original.pdf");
  copyFileSync(corpus("benign/multi-page-10.pdf"), file);
  const page = await launch(file);
  await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
  expect(await landscape(page)).toBe(false);
  return { page, file };
}

/** Turns page 1 through the app's IPC, in the document whose pages are shown. */
async function rotateFirstPage(page: Page) {
  const doc = Number(await firstPage(page).getAttribute("data-doc"));
  await page.evaluate(async (doc) => {
    await (window as unknown as TauriWindow).__TAURI_INTERNALS__.invoke("apply_edit", {
      args: { doc, edit: { kind: "rotatePages", pages: [0], by: "cw90" } },
    });
  }, doc);
  await expect(unsavedTab(page)).toBeVisible();
  await expect.poll(() => landscape(page)).toBe(true);
}

test.describe("saving", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-saving-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("an edit saved as another file is in that file, and the original is as it was", async ({ launch }) => {
    const { page, file } = await openCopy(launch, folder);
    const before = hash(file);
    await rotateFirstPage(page);

    await page.keyboard.press("Control+Shift+S");
    const copy = path.join(folder, "rotated.pdf");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    // The tab now stands for the copy, with nothing left to save.
    await expect(page.getByRole("tab", { name: "rotated.pdf" })).toBeVisible();
    expect(hash(file)).toBe(before);

    await quit(page);
    const reopened = await launch(copy);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    expect(await landscape(reopened)).toBe(true);
  });

  test("Ctrl+S writes the edit into the file itself", async ({ launch }) => {
    const { page, file } = await openCopy(launch, folder);
    await rotateFirstPage(page);

    await page.keyboard.press("Control+S");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await expect(page.getByRole("tab", { name: "original.pdf" })).toBeVisible();

    await quit(page);
    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    expect(await landscape(reopened)).toBe(true);
  });

  test("closing a tab with unsaved changes asks first", async ({ launch }) => {
    const { page, file } = await openCopy(launch, folder);
    const before = hash(file);
    await rotateFirstPage(page);

    await page.keyboard.press("Control+W");
    const question = page.getByRole("dialog", { name: strings.saving.askTitle });
    await question.getByRole("button", { name: strings.saving.cancel }).click();
    await expect(unsavedTab(page)).toBeVisible();

    await page.keyboard.press("Control+W");
    await question.getByRole("button", { name: strings.saving.discard }).click();
    await expect(page.getByRole("heading", { level: 1, name: strings.empty.title })).toBeVisible();
    expect(hash(file)).toBe(before);
  });

  test("the window asks before it closes with unsaved changes, and can save them", async ({ launch }) => {
    const { page, file } = await openCopy(launch, folder);
    await rotateFirstPage(page);

    await closeAppWindow(page);
    const question = page.getByRole("dialog", { name: strings.saving.askTitle });
    await expect(question).toContainText(strings.saving.askOne("original.pdf"));
    await question.getByRole("button", { name: strings.saving.cancel }).click();
    await expect(question).toHaveCount(0);
    expect(appRunning(page)).toBe(true);

    await closeAppWindow(page);
    await question.getByRole("button", { name: strings.saving.save, exact: true }).click();
    await expect.poll(() => appRunning(page), { timeout: 15_000 }).toBe(false);

    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    expect(await landscape(reopened)).toBe(true);
  });
});
