// Sanitized copy (B3-15) in the real app: a copy without active content or hidden data,
// written where the user says in the system's save dialog, which then opens without security warnings;
// the original document file is not touched.
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.sanitize;
const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

test.describe("sanitize copy", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-sanitize-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("the sanitized copy has no active content, opens without security warning, and original is untouched", async ({ launch }) => {
    const original = corpus("malicious/openaction-js.pdf");
    const before = sha256(original);
    const page = await launch(original);

    // Initial page shows the active content banner
    const banner = page.getByRole("region", { name: strings.banner.label });
    await expect(banner).toBeVisible();

    // Open More Menu -> 分享前淨化…
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(strings.menu.sanitizeCopy) }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toBeVisible();

    // Start sanitization
    await dialog.getByRole("button", { name: t.start }).click();

    const copy = path.join(folder, "sanitized.pdf");
    const report = await answerFileDialog(page, { path: copy });

    // Success dialog shows done title and report
    const doneDialog = page.getByRole("dialog", { name: t.doneTitle });
    await expect(doneDialog).toBeVisible();
    await doneDialog.getByRole("button", { name: t.close }).click();

    expect(readdirSync(folder), report).toEqual(["sanitized.pdf"]);
    const bytes = readFileSync(copy);
    expect(bytes.subarray(0, 5).toString()).toBe("%PDF-");

    // The document's own file is untouched
    expect(sha256(original)).toBe(before);

    // Opening the copy in a new tab: banner is not visible (active content removed)
    await page.keyboard.press("Control+o");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("tab", { name: /sanitized\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("region", { name: strings.banner.label })).toHaveCount(0);
  });
});
