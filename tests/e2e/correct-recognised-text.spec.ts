// Correcting OCR words in a page's invisible `3 Tr` text layer (B7-12, Issue #749, ADR 0036).
// Edits "Hel1o" -> "Hello" inline on `benign/recognised-text-layer.pdf`, checks search before save,
// saves a copy, quits and reopens the saved copy, and verifies negative controls stay intact.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

test.describe("correcting recognised text (#749, B7-12)", () => {
  let folder: string;

  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-correct-ocr-"));
  });

  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("edits 3 Tr word inline, updates search immediately, and persists after saving a copy", async ({
    launch,
  }) => {
    const page = await launch(corpus("benign/recognised-text-layer.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute(
      "data-state",
      "ready",
    );

    // Enter correction mode from the "..." menu.
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: strings.menu.correctRecognisedText }).click();

    const helloBtn = page.getByRole("button", { name: strings.ocr.correct.wordLabel("Hel1o") });
    await expect(helloBtn).toBeVisible();
    await expect(page.getByRole("button", { name: strings.ocr.correct.wordLabel("W0rld") })).toBeVisible();

    // Click the word to edit inline without modal dialogs.
    await helloBtn.click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByText(strings.ocr.correct.notice)).toBeVisible();
    const editor = page.getByRole("textbox", { name: strings.ocr.correct.editorLabel });
    await expect(editor).toHaveValue("Hel1o");

    await editor.fill("Hello");
    await page.keyboard.press("Enter");

    await expect(editor).toHaveCount(0);
    await expect(page.getByRole("button", { name: strings.ocr.correct.wordLabel("Hello") })).toBeVisible();

    // Exit correction mode with Esc, then verify search reflects the edit before save.
    await page.keyboard.press("Escape");
    await expect(page.getByRole("button", { name: strings.ocr.correct.wordLabel("Hello") })).toHaveCount(0);

    await page.keyboard.press("Control+f");
    const search = page.getByRole("search", { name: strings.search.label });
    const searchInput = search.getByRole("textbox", { name: strings.search.placeholder });
    const searchStatus = search.getByRole("status");

    await searchInput.fill("Hello");
    await expect(searchStatus).toHaveText(strings.search.count(1, 1));
    await searchInput.fill("Hel1o");
    await expect(searchStatus).toHaveText(strings.search.noResults("Hel1o"));

    // Save to a separate copy so the tracked corpus file is untouched.
    const copy = path.join(folder, "corrected-ocr.pdf");
    await page.keyboard.press("Control+Shift+S");
    const report = await answerFileDialog(page, { path: copy });
    expect(readFileSync(copy).subarray(0, 5).toString(), report).toBe("%PDF-");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);

    await quit(page);

    // Relaunch on the saved copy and confirm search, negative controls, and correction mode persist.
    const reopened = await launch(copy);
    await expect(reopened.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute(
      "data-state",
      "ready",
    );

    await reopened.keyboard.press("Control+f");
    const reopenedSearch = reopened.getByRole("search", { name: strings.search.label });
    const reopenedInput = reopenedSearch.getByRole("textbox", { name: strings.search.placeholder });
    const reopenedStatus = reopenedSearch.getByRole("status");

    for (const term of ["Hello", "W0rld", "VisibleKeep", "ClipKeep", "AlphaZeroKeep"]) {
      await reopenedInput.fill(term);
      await expect(reopenedStatus).toHaveText(strings.search.count(1, 1));
    }
    await reopenedInput.fill("Hel1o");
    await expect(reopenedStatus).toHaveText(strings.search.noResults("Hel1o"));

    await reopened.getByRole("button", { name: strings.toolbar.more }).click();
    await reopened.getByRole("menuitem", { name: strings.menu.correctRecognisedText }).click();
    await expect(
      reopened.getByRole("button", { name: strings.ocr.correct.wordLabel("Hello") }),
    ).toBeVisible();
    await expect(
      reopened.getByRole("button", { name: strings.ocr.correct.wordLabel("W0rld") }),
    ).toBeVisible();
  });
});
