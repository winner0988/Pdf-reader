// Security redaction end-to-end tests (ADR 0016, B2-21, B2-22).
// Verifies marking, confirmation dialog with out-of-scope caveats, applying, undo/redo,
// saving whole rewritten document with verification, reopening, and refusal on signed/restricted PDFs.

import { copyFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, test } from "./app";
import { findOnScreen } from "./search";

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });
const marks = (page: Page) => page.locator("[data-page-annotations] [data-annotation=redaction]");

test.describe("security redaction (ADR 0016, B2-22)", () => {
  let folder: string;

  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-redaction-"));
  });

  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("mark, apply, undo, redo, save, and reopen without the redacted secret", async ({ launch }) => {
    const file = path.join(folder, "redaction-sample.pdf");
    copyFileSync(corpus("benign/redaction-sample.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(marks(page)).toHaveCount(0);

    // 1. Search for SECRET-ALPHA-123 on the page.
    const hitBox = await findOnScreen(page, "SECRET-ALPHA-123");
    await page.keyboard.press("Escape");

    // 2. Mark it with the redaction tool: drag a box over what search found. A link, a note and a
    // field lie on the same line (redacting the line must remove them too), so a right click there
    // lands on the link, not the page; the context menu is tested on a page without them below.
    const redactBtn = page.locator("[data-redact-button]");
    await redactBtn.click();
    await expect(page.getByRole("contentinfo")).toContainText(strings.redaction.toolOn);
    await page.mouse.move(hitBox.x - 2, hitBox.y - 2);
    await page.mouse.down();
    await page.mouse.move(hitBox.x + hitBox.width + 2, hitBox.y + hitBox.height + 2, { steps: 5 });
    await page.mouse.up();
    await page.keyboard.press("Escape");

    // The redaction mark is placed on the page.
    await expect(marks(page)).toHaveCount(1);

    // 3. Open the sidebar redaction tab.
    await page.getByRole("tab", { name: strings.redaction.tab }).click();
    const panel = page.getByRole("tabpanel", { name: strings.redaction.tab });
    await expect(panel).toBeVisible();
    await expect(panel.getByText("第 1 頁")).toBeVisible();

    // 4. Click Apply button to trigger the confirmation dialog.
    const applyBtn = panel.getByRole("button", { name: strings.redaction.apply });
    await expect(applyBtn).toBeEnabled();
    await applyBtn.click();

    // 5. Verify RedactionConfirmDialog and out-of-scope caveats.
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText(strings.redaction.dialog.title)).toBeVisible();
    for (const caveat of strings.redaction.dialog.scopeCaveats) {
      await expect(dialog.getByText(caveat)).toBeVisible();
    }

    // Cancel first to verify non-destructive dismissal.
    await dialog.getByRole("button", { name: strings.redaction.dialog.cancel }).click();
    await expect(dialog).toBeHidden();
    await expect(marks(page)).toHaveCount(1);

    // Click Apply again and confirm.
    await applyBtn.click();
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: strings.redaction.dialog.confirm }).click();
    await expect(dialog).toBeHidden();

    // 6. Applying is confirmed: mark is gone from overlay, status bar hint shows applied.
    await expect(marks(page)).toHaveCount(0);
    await expect(page.getByRole("contentinfo")).toContainText(strings.redaction.applied);

    // 7. Undo with Ctrl+Z brings the mark and original content back.
    await page.keyboard.press("Control+z");
    await expect(marks(page)).toHaveCount(1);

    // Redo with Ctrl+Y applies again.
    await page.keyboard.press("Control+y");
    await expect(marks(page)).toHaveCount(0);

    // 8. Save with Ctrl+S: verifies and rewrites the file whole.
    await page.keyboard.press("Control+s");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.redactionVerified);

    // 9. Inspect the saved file bytes on disk directly.
    await quit(page);
    const fileBytes = readFileSync(file);
    const textBytes = fileBytes.toString("latin1");

    // Redacted text, note, form field and link are thoroughly eliminated.
    expect(textBytes.includes("SECRET-ALPHA-123 inside")).toBe(false);
    expect(textBytes.includes("SECRET-ALPHA-123 in a note")).toBe(false);
    expect(textBytes.includes("SECRET-ALPHA-123 in a field")).toBe(false);
    expect(textBytes.includes("secret-alpha-123.example")).toBe(false);

    // Out-of-scope items declared in the dialog (bookmark and title) remain as specified.
    expect(textBytes.includes("SECRET-ALPHA-123 bookmark")).toBe(true);
    expect(textBytes.includes("SECRET-ALPHA-123 title")).toBe(true);

    // 10. Reopen the saved file in a fresh app instance.
    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");

    // Searching for the secret finds 0 hits.
    await reopened.keyboard.press("Control+f");
    const reopenSearch = reopened.getByRole("textbox", { name: strings.search.placeholder });
    await reopenSearch.fill("SECRET-ALPHA-123");
    await reopenSearch.press("Enter");
    const searchStatus = reopened.getByRole("search").getByRole("status");
    await expect(searchStatus).toHaveText(strings.search.noResults("SECRET-ALPHA-123"));
  });

  test("selected text is marked from the context menu", async ({ launch }) => {
    const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    const hitBox = await findOnScreen(page, "Privacy");
    await page.keyboard.press("Escape");

    await page.mouse.dblclick(hitBox.x + hitBox.width / 2, hitBox.y + hitBox.height / 2);
    await page.mouse.click(hitBox.x + hitBox.width / 2, hitBox.y + hitBox.height / 2, { button: "right" });
    await page.getByRole("menuitem", { name: strings.redaction.markSelection }).click();
    await expect(marks(page)).toHaveCount(1);
  });

  test("redaction is disabled on a signed document", async ({ launch }) => {
    const page = await launch(corpus("benign/signed.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    // The toolbar button is disabled.
    const redactBtn = page.locator("[data-redact-button]");
    await expect(redactBtn).toBeDisabled();

    // The sidebar redaction panel explains that signed documents cannot be redacted.
    await page.getByRole("tab", { name: strings.redaction.tab }).click();
    const panel = page.getByRole("tabpanel", { name: strings.redaction.tab });
    await expect(panel.getByText(strings.redaction.signedDocument)).toBeVisible();
  });

  test("redaction is disabled on a document whose author forbids modifications", async ({ launch }) => {
    const page = await launch(corpus("benign/restricted-no-modify.pdf"));
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");

    // The toolbar button is disabled.
    const redactBtn = page.locator("[data-redact-button]");
    await expect(redactBtn).toBeDisabled();

    // The sidebar redaction panel explains permissions restriction.
    await page.getByRole("tab", { name: strings.redaction.tab }).click();
    const panel = page.getByRole("tabpanel", { name: strings.redaction.tab });
    await expect(panel.getByText(strings.redaction.notAllowed)).toBeVisible();
  });

  test("find personal data, mark it, apply, save, and reopen without it (#302)", async ({ launch }) => {
    const find = strings.redaction.findPersonalDataDialog;
    const file = path.join(folder, "personal-data.pdf");
    copyFileSync(corpus("benign/personal-data.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(marks(page)).toHaveCount(0);

    await page.getByRole("tab", { name: strings.redaction.tab }).click();
    const panel = page.getByRole("tabpanel", { name: strings.redaction.tab });
    await panel.getByRole("button", { name: strings.redaction.findPersonalData }).click();

    const dialog = page.getByRole("dialog");
    await expect(dialog.getByText(find.title)).toBeVisible();
    await dialog.getByRole("button", { name: find.startSearch }).click();

    // Every category but the keyword is on by default. The sample has 11 valid items; the ones
    // with a wrong check digit or length are left out. Only masked previews are listed.
    await expect(dialog.getByText(find.matchesFound(11))).toBeVisible();
    await expect(dialog.getByText("A12****789")).toHaveCount(2);
    await expect(dialog.getByText("a***e@example.com")).toBeVisible();
    await expect(dialog.getByText("A123456789")).toHaveCount(0);

    // One mark per line of each item: the ID wrapped over two lines takes two.
    await dialog.getByRole("button", { name: find.markSelected(11) }).click();
    await expect(dialog).toBeHidden();
    await expect(marks(page)).toHaveCount(12);

    await panel.getByRole("button", { name: strings.redaction.apply }).click();
    await expect(dialog.getByText(strings.redaction.dialog.title)).toBeVisible();
    await dialog.getByRole("button", { name: strings.redaction.dialog.confirm }).click();
    await expect(dialog).toBeHidden();
    await expect(marks(page)).toHaveCount(0);

    // Saving checks the redaction (ADR 0016); the reopened file no longer has the found text.
    await page.keyboard.press("Control+s");
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.redactionVerified);
    await quit(page);

    const reopened = await launch(file);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    await reopened.keyboard.press("Control+f");
    const searchInput = reopened.getByRole("textbox", { name: strings.search.placeholder });
    const searchStatus = reopened.getByRole("search").getByRole("status");
    for (const gone of ["0912-345-678", "alice.sample@example.com", "4111-1111-1111-1111"]) {
      await searchInput.fill(gone);
      await searchInput.press("Enter");
      await expect(searchStatus).toHaveText(strings.search.noResults(gone));
    }
    // What was not found stays: the card number that fails the Luhn check.
    await searchInput.fill("4111-1111-1111-1112");
    await searchInput.press("Enter");
    await expect(searchStatus).toHaveText(strings.search.count(1, 1));
  });
});
