// Saving a searchable copy of a scanned PDF (B4-08, Issue #416, ADR 0028) in the real app:
// opens a scan with auto-OCR off, saves a searchable copy via the dedicated dialog (which runs
// OCR in the sandboxed worker and writes an invisible text layer into a new file), verifies the
// original file is untouched and the copy is not opened automatically, then opens the copy (still
// with auto-OCR off) and verifies search finds text directly from the embedded text layer.
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, documentTabs, expect, test } from "./app";

const t = strings.ocr.searchable;
const SCAN = corpus("benign/scanned-text.pdf");

const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

function dataDirWithEnglishManualOcr(): string {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-searchable-data-"));
  const settings = { version: 1, theme: "system", recordRecentFiles: true, ocrAuto: false, ocrLanguage: "eng" };
  writeFileSync(path.join(folder, "settings.json"), JSON.stringify(settings));
  return folder;
}

test.describe("searchable PDF copy", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-searchable-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("recognises a scan, writes an invisible text layer to a copy, leaves original untouched, and copy is searchable without OCR", async ({
    launch,
  }) => {
    const before = sha256(SCAN);
    const page = await launch(SCAN, { dataDir: dataDirWithEnglishManualOcr() });
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );

    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(strings.menu.searchableCopy) }).click();

    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toContainText("English（eng）");
    await expect(dialog).toContainText(t.pagesSummary(1));

    await dialog.getByRole("button", { name: t.start }).click();

    // OCR completes and the Windows save dialog appears; answer it with our target path.
    const copy = path.join(folder, "searchable.pdf");
    const report = await answerFileDialog(page, { path: copy });

    await expect(dialog).toContainText(t.doneTitle, { timeout: 60_000 });
    await expect(dialog).toContainText(t.doneAdded(1));
    await dialog.getByRole("button", { name: t.doneButton }).click();
    await expect(dialog).toHaveCount(0);

    expect(readdirSync(folder), report).toEqual(["searchable.pdf"]);
    const bytes = readFileSync(copy);
    expect(bytes.subarray(0, 5).toString()).toBe("%PDF-");
    // Original file untouched; copy is not automatically opened.
    expect(sha256(SCAN)).toBe(before);
    await expect(documentTabs(page)).toHaveCount(1);

    // Open the copy in a new tab: with ocrAuto=false, search must succeed purely from the
    // embedded invisible text layer (and no OCR note is shown).
    await page.keyboard.press("Control+o");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("tab", { name: /searchable\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).last()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await expect(page.getByTestId("ocr-note")).toHaveCount(0);

    await page.keyboard.press("Control+f");
    const input = page.getByRole("textbox", { name: strings.search.placeholder });
    await input.fill("secret");
    await input.press("Enter");
    await expect(page.getByRole("search").getByRole("status")).toHaveText(strings.search.count(1, 1), {
      timeout: 15_000,
    });
  });
});
