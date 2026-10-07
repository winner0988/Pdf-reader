// Recognising the text of scanned pages (B2-10, ADR 0015) in the real app: the worker reads a
// scan in the background, in its own sandbox, with the language data that came with the app; the
// pages then have text. `benign/scanned-text.pdf` is one picture of two lines of text and nothing
// else (its lettering is blocky, which the English data reads exactly: the data of Traditional
// Chinese, which the app chooses by itself, reads it badly, so these tests choose English).
import { copyFileSync, existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, dataDir, ROOT, expect, test } from "./app";

const t = strings.ocr;
const SCAN = corpus("benign/scanned-text.pdf");
const BUNDLED = path.join(ROOT, "src-tauri", "resources", "tessdata");

/** A data folder whose settings choose English, and whether recognising starts by itself. */
function data(ocrAuto: boolean): string {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-ocr-"));
  const settings = { version: 1, theme: "system", recordRecentFiles: true, ocrAuto, ocrLanguage: "eng" };
  writeFileSync(path.join(folder, "settings.json"), JSON.stringify(settings));
  return folder;
}

async function open(launch: (file?: string, options?: { dataDir?: string }) => Promise<Page>, ocrAuto = true) {
  const page = await launch(SCAN, { dataDir: data(ocrAuto) });
  await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
  // Copied text is recorded instead of put on this computer's clipboard.
  await page.evaluate(() => {
    const copied: string[] = [];
    Object.assign(window, { copied });
    navigator.clipboard.writeText = async (text: string) => {
      copied.push(text);
    };
  });
  const copied = () => page.evaluate(() => (window as unknown as { copied: string[] }).copied.at(-1));
  return { page, copied };
}

/** Searches for `query`; what the search bar says once it is done. */
async function search(page: Page, query: string) {
  await page.keyboard.press("Control+f");
  const input = page.getByRole("textbox", { name: strings.search.placeholder });
  await input.fill(query);
  await input.press("Enter");
  return page.getByRole("search").getByRole("status");
}

test("a scanned page is read in the background, and then it has text: marked, found and copied", async ({ launch }) => {
  const { page, copied } = await open(launch);

  // Nothing was asked for: the main process looked at the pages by itself, found the scan and had
  // the worker read it; the status bar says the text of the page shown is recognised.
  await expect(page.getByTestId("ocr-note")).toHaveText(t.pageNote, { timeout: 60_000 });

  // The text is the picture's, in the page's own space: search finds a word and marks it.
  const status = await search(page, "secret");
  await expect(status).toHaveText(strings.search.count(1, 1), { timeout: 15_000 });
  const hit = page.locator("[data-highlights] polygon:not([data-current])");
  await expect(hit).toHaveCount(1);
  const box = await hit.boundingBox();
  if (!box) throw new Error("the hit is not on screen");

  // And it can be selected and copied like any text.
  await page.mouse.dblclick(box.x + box.width / 2, box.y + box.height / 2);
  await page.keyboard.press("Control+c");
  await expect.poll(copied).toBe("SECRET");
});

test("left to the user, nothing is read until they ask from the menu", async ({ launch }) => {
  const { page } = await open(launch, false);
  // The page has no text: search says the document has no text layer, and nothing is marked.
  const status = await search(page, "secret");
  await expect(status).toHaveText(strings.search.noTextLayer, { timeout: 15_000 });
  await expect(page.getByTestId("ocr-note")).toHaveCount(0);
  await page.keyboard.press("Escape");

  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: t.menu }).click();
  await expect(page.getByTestId("ocr-note")).toHaveText(t.pageNote, { timeout: 60_000 });
  // Done: the reader says so, and the search that was searched again would find it too.
  await expect(page.getByRole("contentinfo")).not.toContainText(t.running(0, 1));
  const again = await search(page, "paper");
  await expect(again).toHaveText(strings.search.count(1, 1), { timeout: 15_000 });
});

test("a language is imported from a file the main process asks for, checked, and removed again", async ({ launch }) => {
  const page = await launch();
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-language-"));
  const imported = path.join(dataDir(page), "tessdata");

  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: strings.menu.settings }).click();
  const dialog = page.getByRole("dialog", { name: strings.settings.title });
  const s = t.settings;
  await expect(dialog.getByRole("heading", { name: s.title })).toBeVisible();
  // What came with the app: the language the app chooses first, then both.
  const language = dialog.getByRole("combobox", { name: s.language });
  await expect(language.locator("option")).toHaveText([s.automatic("繁體中文（chi_tra）"), "繁體中文（chi_tra）", "English（eng）"]);

  const importFile = async (file: string) => {
    await dialog.getByRole("button", { name: s.importButton }).click();
    await answerFileDialog(page, { path: file });
  };

  // Not language data, and a name that is no language's.
  const notData = path.join(folder, "xyz.traineddata");
  writeFileSync(notData, "this is not language data");
  await importFile(notData);
  await expect(dialog.getByText(s.refused.notLanguageData)).toBeVisible();
  const badName = path.join(folder, "1abc.traineddata");
  copyFileSync(path.join(BUNDLED, "eng.traineddata"), badName);
  await importFile(badName);
  await expect(dialog.getByText(s.refused.badName)).toBeVisible();
  // One that came with the app cannot be replaced.
  const sameName = path.join(folder, "eng.traineddata");
  copyFileSync(path.join(BUNDLED, "chi_tra.traineddata"), sameName);
  await importFile(sameName);
  await expect(dialog.getByText(s.refused.nameTaken)).toBeVisible();
  expect(existsSync(path.join(imported, "xyz.traineddata"))).toBe(false);

  // Real data under another name is imported: it is listed, can be chosen, and is kept.
  const german = path.join(folder, "deu.traineddata");
  copyFileSync(path.join(BUNDLED, "eng.traineddata"), german);
  await importFile(german);
  await expect(dialog.getByText(s.imported)).toBeVisible();
  await expect(language.locator("option")).toContainText(["Deutsch（deu）"]);
  expect(readFileSync(path.join(imported, "deu.traineddata")).length).toBe(readFileSync(german).length);

  await language.selectOption("deu");
  const settingsFile = path.join(dataDir(page), "settings.json");
  await expect.poll(() => JSON.parse(readFileSync(settingsFile, "utf8")).ocrLanguage).toBe("deu");

  // Removing it leaves the choice to the app again.
  await dialog.getByRole("button", { name: s.remove("Deutsch（deu）") }).click();
  await expect(dialog.getByText(s.removed)).toBeVisible();
  await expect(language).toHaveValue("");
  expect(existsSync(path.join(imported, "deu.traineddata"))).toBe(false);
  await expect.poll(() => JSON.parse(readFileSync(settingsFile, "utf8")).ocrLanguage).toBe(null);
});
