// The signatures of a document in the real app (B2-14, ADR 0014): verified offline by the
// document's own worker, in its sandbox, with Windows' cryptography. The corpus's two signed files
// are signed with a self-signed test certificate: their signatures hold, and nobody vouches for
// the signer.
import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, dataDir, expect, quit, test } from "./app";

const t = strings.signatures;
const SIGNER = "PDF Reader test corpus signer (NOT TRUSTED)";

test.describe("digital signatures", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-signatures-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("a signed document says its signature holds, but that nobody vouches for the signer", async ({ launch }) => {
    const page = await launch(corpus("benign/signed.pdf"));

    const banner = page.getByRole("region", { name: t.label });
    await expect(banner).toContainText(t.summaryOne.unconfirmed);
    await expect(banner).toHaveAttribute("data-signatures", "unconfirmed");
    // The page itself is shown as usual.
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

    await banner.getByRole("button", { name: t.details }).click();
    const panel = page.getByRole("complementary", { name: t.detailsTitle });
    await expect(panel).toContainText("Signature1");
    await expect(panel).toContainText(t.status.unconfirmed);
    await expect(panel).toContainText(SIGNER);
    await expect(panel).toContainText("2026-01-01 00:00:00 UTC");
    await expect(panel).toContainText(t.timeNote);
    await expect(panel).toContainText(t.detailsNote);
    // Closing the panel puts focus back on the banner's button.
    await panel.getByRole("button", { name: t.detailsClose }).click();
    await expect(panel).toHaveCount(0);
    await expect(banner.getByRole("button", { name: t.details })).toBeFocused();
    // The banner can be closed too.
    await banner.getByRole("button", { name: t.dismiss }).click();
    await expect(banner).toHaveCount(0);
  });

  test("a certifying signature says what it allows", async ({ launch }) => {
    const page = await launch(corpus("benign/signed-docmdp-p1.pdf"));

    const banner = page.getByRole("region", { name: t.label });
    await expect(banner).toContainText(t.summaryOne.unconfirmed);
    await banner.getByRole("button", { name: t.details }).click();
    const panel = page.getByRole("complementary", { name: t.detailsTitle });
    await expect(panel).toContainText(t.certification);
    await expect(panel).toContainText(t.certificationLevel.noChanges);
    // Nothing changed after it: no warning.
    await expect(panel).not.toContainText(t.certificationBroken);
  });

  test("a signature whose signed bytes were changed is invalid", async ({ launch }) => {
    const bytes = readFileSync(corpus("benign/signed.pdf"));
    // One letter of the page's text, inside what the signature covers.
    bytes[bytes.indexOf("Signed sample")] = "s".charCodeAt(0);
    const changed = path.join(folder, "changed.pdf");
    writeFileSync(changed, bytes);
    const page = await launch(changed);

    const banner = page.getByRole("region", { name: t.label });
    await expect(banner).toContainText(t.summaryOne.invalid);
    await expect(banner).toHaveAttribute("data-signatures", "invalid");
    await banner.getByRole("button", { name: t.details }).click();
    const panel = page.getByRole("complementary", { name: t.detailsTitle });
    await expect(panel).toContainText(t.status.invalid);
    // A signature that does not hold names nobody.
    await expect(panel).not.toContainText(SIGNER);
  });

  test("a document without signatures has no signature banner", async ({ launch }) => {
    const page = await launch(corpus("benign/multi-page-10.pdf"));

    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");
    await expect(page.getByRole("region", { name: t.label })).toHaveCount(0);
  });
});

test.describe("reusable signatures (B6-02)", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-saved-signatures-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("draws, places on pages, persists across launches encrypted with DPAPI, deletes and cleans up", async ({
    launch,
  }) => {
    const file = path.join(folder, "sample.pdf");
    copyFileSync(corpus("benign/single-page.pdf"), file);

    // 1. Open benign PDF copy, select "簽名…" from stamp menu, draw with mouse on pad, click "儲存" -> count = 1
    const page = await launch(file);
    await expect(page.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

    await page.locator("[data-stamp-button]").click();
    await page.getByRole("menuitem", { name: strings.annotations.signature }).click();
    const dialog = page.getByRole("dialog", { name: strings.savedSignatures.dialogTitle });
    await expect(dialog).toBeVisible();

    // Verify role="note" security notice is visible
    await expect(dialog.getByRole("note")).toContainText(strings.savedSignatures.note);

    // Draw on pad
    const pad = dialog.getByLabel(strings.savedSignatures.padAriaLabel);
    const padBox = await pad.boundingBox();
    if (!padBox) throw new Error("pad is not visible");
    await page.mouse.move(padBox.x + 30, padBox.y + 30);
    await page.mouse.down();
    await page.mouse.move(padBox.x + 120, padBox.y + 70);
    await page.mouse.up();

    await dialog.getByRole("button", { name: strings.savedSignatures.saveDrawn }).click();
    const placeButtons = dialog.getByRole("button", { name: new RegExp(`^${strings.savedSignatures.clickToPlace}`) });
    await expect(placeButtons).toHaveCount(1);

    // 2. Click it, click page -> places stamp. Save document.
    await placeButtons.first().click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole("contentinfo")).toContainText(strings.annotations.placeSignature);

    const sheet = page.getByRole("img", { name: strings.canvas.page(1) });
    const sheetBox = await sheet.boundingBox();
    if (!sheetBox) throw new Error("sheet is not visible");
    await page.mouse.click(sheetBox.x + sheetBox.width * 0.5, sheetBox.y + sheetBox.height * 0.5);

    await expect(page.locator("[data-page-annotations] [data-annotation=stamp]")).toHaveCount(1);
    await page.keyboard.press("Control+s");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);

    // 3. quit, open another copy using same dataDir -> signature is still in "簽名…", place it and save
    const data = dataDir(page);
    await quit(page);

    const anotherFile = path.join(folder, "another.pdf");
    copyFileSync(corpus("benign/single-page.pdf"), anotherFile);
    const again = await launch(anotherFile, { dataDir: data });
    await expect(again.getByRole("img", { name: strings.canvas.page(1) })).toHaveAttribute("data-state", "ready");

    await again.locator("[data-stamp-button]").click();
    await again.getByRole("menuitem", { name: strings.annotations.signature }).click();
    const dialogAgain = again.getByRole("dialog", { name: strings.savedSignatures.dialogTitle });
    await expect(dialogAgain).toBeVisible();

    const placeButtonsAgain = dialogAgain.getByRole("button", { name: new RegExp(`^${strings.savedSignatures.clickToPlace}`) });
    await expect(placeButtonsAgain).toHaveCount(1);
    await placeButtonsAgain.first().click();
    await expect(dialogAgain).toHaveCount(0);

    const againSheet = again.getByRole("img", { name: strings.canvas.page(1) });
    const againBox = await againSheet.boundingBox();
    if (!againBox) throw new Error("againSheet is not visible");
    await again.mouse.click(againBox.x + againBox.width * 0.4, againBox.y + againBox.height * 0.4);
    await expect(again.locator("[data-page-annotations] [data-annotation=stamp]")).toHaveCount(1);
    await again.keyboard.press("Control+s");
    await expect(again.getByRole("contentinfo")).toContainText(strings.saving.saved);

    // 4. Check dataDir's signatures\ directory -> exactly 1 .sig file, starts with non-PNG bytes
    const sigDir = path.join(data, "signatures");
    const sigFiles = readdirSync(sigDir).filter((f) => f.endsWith(".sig"));
    expect(sigFiles).toHaveLength(1);
    const sigBytes = readFileSync(path.join(sigDir, sigFiles[0]!));
    // PNG starts with 0x89 0x50 0x4E 0x47
    expect(sigBytes.subarray(0, 4)).not.toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47]));

    // 5. Delete it in dialog -> signatures\ has no .sig files
    await again.locator("[data-stamp-button]").click();
    await again.getByRole("menuitem", { name: strings.annotations.signature }).click();
    const deleteBtn = dialogAgain.getByRole("button", { name: strings.savedSignatures.delete }).first();
    await deleteBtn.click();
    await dialogAgain.getByRole("button", { name: strings.savedSignatures.confirmDelete }).click();
    await expect(dialogAgain.getByRole("button", { name: new RegExp(`^${strings.savedSignatures.clickToPlace}`) })).toHaveCount(0);
    const leftFiles = readdirSync(sigDir).filter((f) => f.endsWith(".sig"));
    expect(leftFiles).toHaveLength(0);

    // 6. "輸入" type name and save, "匯入圖片…" select images/stamp-exif.jpg via answerFileDialog -> count = 2
    await dialogAgain.getByRole("button", { name: strings.savedSignatures.typeTab }).click();
    await dialogAgain.getByPlaceholder(strings.savedSignatures.typePlaceholder).fill("Alice User");
    await dialogAgain.getByRole("button", { name: strings.savedSignatures.saveTyped }).click();
    await expect(dialogAgain.getByRole("button", { name: new RegExp(`^${strings.savedSignatures.clickToPlace}`) })).toHaveCount(1);

    await dialogAgain.getByRole("button", { name: strings.savedSignatures.importTab }).click();
    const importDialogPromise = answerFileDialog(again, { path: corpus("images/stamp-exif.jpg") });
    await dialogAgain.getByRole("button", { name: strings.savedSignatures.importButton }).click();
    await importDialogPromise;
    await expect(dialogAgain.getByRole("button", { name: new RegExp(`^${strings.savedSignatures.clickToPlace}`) })).toHaveCount(2);

    await again.keyboard.press("Escape");
    await expect(dialogAgain).toHaveCount(0);

    // 7. Open settings, click "全部刪除" -> signatures\ directory is gone or empty
    await again.getByRole("button", { name: strings.toolbar.more }).click();
    await again.getByRole("menuitem", { name: strings.menu.settings }).click();
    const settingsDialog = again.getByRole("dialog", { name: strings.settings.title });
    await expect(settingsDialog).toBeVisible();

    await settingsDialog.getByRole("button", { name: strings.settings.signatures.removeAll }).click();
    await settingsDialog.getByRole("button", { name: strings.settings.signatures.confirmRemoveAll }).click();
    await expect(settingsDialog.getByRole("status")).toContainText(strings.settings.signatures.allRemoved);

    expect(!existsSync(sigDir) || readdirSync(sigDir).filter((f) => f.endsWith(".sig")).length === 0).toBe(true);
  });
});
