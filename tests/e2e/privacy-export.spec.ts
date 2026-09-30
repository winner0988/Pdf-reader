// The privacy export (B2-03) in the real app: a copy without the document's metadata, written
// where the user says in the system's save dialog (answered through UI Automation), which then
// opens like any PDF; the document's own file is not touched.
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.privacyExport;
/** The author the sample names in every kind of metadata (tests/corpus/generate.py). */
const AUTHOR = Buffer.from("Jane Q. Private-Author");

const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

test.describe("privacy export", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-privacy-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("the copy has no metadata, opens again, and the document's file is untouched", async ({ launch }) => {
    const original = corpus("benign/metadata-full.pdf");
    const before = sha256(original);
    const page = await launch(original);
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(strings.menu.privacyExport) }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toContainText(t.signatures);
    await dialog.getByRole("button", { name: t.start }).click();

    const copy = path.join(folder, "clean.pdf");
    const report = await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(t.done);
    expect(readdirSync(folder), report).toEqual(["clean.pdf"]);
    const bytes = readFileSync(copy);
    expect(bytes.subarray(0, 5).toString()).toBe("%PDF-");
    expect(bytes.includes(AUTHOR)).toBe(false);
    // The document's own file, and its tab, are as they were.
    expect(sha256(original)).toBe(before);
    await expect(page.getByRole("tab", { name: /metadata-full\.pdf/ })).toHaveAttribute("aria-selected", "true");

    // The copy opens like any PDF, in a tab of its own.
    await page.keyboard.press("Control+o");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("tab", { name: /clean\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).last()).toHaveAttribute(
      "data-state",
      "ready",
    );
  });
});
