// Page marks and Bates numbering (B3-16) in the real app: a copy with page numbers
// and Bates numbers, written where the user says in the system's save dialog (answered through UI Automation),
// which then opens like any PDF; the numbers are found by search, and the document's own file is not touched.
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.pageMarks;

const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

test.describe("page marks and Bates numbering", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-page-marks-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("writes a copy with page numbers and Bates, opens copy and finds numbers with search, original untouched", async ({ launch }) => {
    const original = corpus("benign/multi-page-10.pdf");
    const before = sha256(original);
    const page = await launch(original);
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );

    // Open "⋯" menu -> "頁碼與 Bates 編號…"
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(strings.menu.pageMarks) }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toContainText(t.signatures);

    // Enable Bates number as well
    await dialog.getByRole("checkbox", { name: t.batesTitle }).click();

    // Start export
    await dialog.getByRole("button", { name: t.start }).click();

    const copy = path.join(folder, "marked.pdf");
    const report = await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(t.done);
    expect(readdirSync(folder), report).toEqual(["marked.pdf"]);

    const bytes = readFileSync(copy);
    expect(bytes.subarray(0, 5).toString()).toBe("%PDF-");

    // The document's own file, and its tab, are unchanged
    expect(sha256(original)).toBe(before);
    await expect(page.getByRole("tab", { name: /multi-page-10\.pdf/ })).toHaveAttribute("aria-selected", "true");

    // Open the copy in a new tab
    await page.keyboard.press("Control+o");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("tab", { name: /marked\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).last()).toHaveAttribute(
      "data-state",
      "ready",
    );

    // Every page of the copy has its page number and its Bates number, found by search: the
    // sample's own text is "Page n of 10", so these words are the marks'.
    await page.keyboard.press("Control+f");
    const search = page.getByRole("search", { name: strings.search.label });
    const searchInput = search.getByRole("textbox", { name: strings.search.placeholder });
    const searchStatus = search.getByRole("status");
    for (const [words, found] of [
      ["共 10 頁", 10],
      ["第 7 頁", 1],
      ["0000", 10],
      ["000010", 1],
    ] as const) {
      await searchInput.fill(words);
      await expect(searchStatus).toHaveText(strings.search.count(1, found));
    }
  });
});
