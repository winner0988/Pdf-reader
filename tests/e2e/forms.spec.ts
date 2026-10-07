// Filling in a form (B2-09) in the real app: every kind of field of a sample form is filled in and
// the document saved as a copy; the copy opens again with the values. A field with scripts says
// that they do not run. Flattening turns the fields into page content, which is saved as another
// file, and the values are still on its page.
import { copyFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.forms;

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });
const unsaved = (page: Page, name: string) =>
  page.getByRole("tab", { name: new RegExp(`^${name}\\s*${strings.tabs.unsaved}$`) });

/** The field boxes on the pages, by what they are called. */
const box = (page: Page, label: string) => page.locator(`[data-page-fields] [aria-label="${label}"]`);

test("every kind of field is filled in, and the saved copy has the values (B2-09)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-forms-"));
  try {
    const file = path.join(folder, "form.pdf");
    copyFileSync(corpus("benign/form-fields.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(box(page, "Your name")).toHaveValue("Jane Q. Public");
    await expect(box(page, "locked")).toHaveAttribute("readonly", "");
    await expect(box(page, "Code of at most five characters")).toHaveAttribute("maxlength", "5");

    await box(page, "Your name").fill("林 小明");
    await box(page, "Your name").press("Enter");
    await expect(unsaved(page, "form\\.pdf")).toBeVisible();
    await box(page, "Notes").fill("one\ntwo");
    await box(page, "Code of at most five characters").fill("AB12Z");
    await box(page, "I agree").check();
    await page.getByRole("radio", { name: t.choice("Size", "Medium") }).check();
    await box(page, "Country").selectOption("JP");
    await box(page, "Fruit").selectOption("cherry");
    // The last text box is only sent once it is left.
    await box(page, "Required").fill("done");
    await box(page, "Required").press("Enter");
    await expect(box(page, "Required")).toHaveValue("done");

    const copy = path.join(folder, "filled.pdf");
    await page.keyboard.press("Control+Shift+S");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await quit(page);

    const reopened = await launch(copy);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    await expect(box(reopened, "Your name")).toHaveValue("林 小明");
    await expect(box(reopened, "Notes")).toHaveValue("one\ntwo");
    await expect(box(reopened, "Code of at most five characters")).toHaveValue("AB12Z");
    await expect(box(reopened, "I agree")).toBeChecked();
    await expect(reopened.getByRole("radio", { name: t.choice("Size", "Medium") })).toBeChecked();
    await expect(reopened.getByRole("radio", { name: t.choice("Size", "Large") })).not.toBeChecked();
    await expect(box(reopened, "Country")).toHaveValue("JP");
    await expect(box(reopened, "Fruit")).toHaveValue("cherry");
    await expect(box(reopened, "Required")).toHaveValue("done");
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("a field with scripts can be filled in, and says that its scripts do not run (B2-09)", async ({ launch }) => {
  const page = await launch(corpus("malicious/field-aa.pdf"));
  await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
  const amount = box(page, "amount");
  await amount.click();
  await expect(page.getByRole("contentinfo")).toContainText(t.scriptNotRun);
  await amount.fill("12.5");
  await amount.press("Enter");
  await expect(unsaved(page, "field-aa\\.pdf")).toBeVisible();
  // Its scripts would have shown an alert; nothing ran.
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(amount).toHaveValue("12.5");
});

test("flattening turns the fields into page content, saved as another file (B2-09)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-flatten-"));
  try {
    const file = path.join(folder, "form.pdf");
    copyFileSync(corpus("benign/form-fields.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await box(page, "Your name").fill("Flattened Name");
    await box(page, "Your name").press("Enter");
    await expect(unsaved(page, "form\\.pdf")).toBeVisible();

    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(t.flatten.replace("…", "")) }).click();
    const dialog = page.getByRole("dialog", { name: t.flattenDialog.title });
    await dialog.getByRole("button", { name: t.flattenDialog.confirm }).click();
    // Where to save the flattened document is asked next; the fields are gone from the page.
    const flattened = path.join(folder, "flattened.pdf");
    await answerFileDialog(page, { path: flattened });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await expect(page.locator("[data-page-fields]")).toHaveCount(0);
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await expect(page.getByRole("menuitem", { name: new RegExp(t.flatten.replace("…", "")) })).toHaveCount(0);
    await page.keyboard.press("Escape");
    await quit(page);

    // The saved file has no form, and the value is on its page, where search finds it.
    const reopened = await launch(flattened);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    await expect(reopened.locator("[data-page-fields]")).toHaveCount(0);
    await reopened.keyboard.press("Control+f");
    const input = reopened.getByRole("textbox", { name: strings.search.placeholder });
    await input.fill("Flattened Name");
    await input.press("Enter");
    await expect(reopened.locator("[data-highlights] polygon")).not.toHaveCount(0);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
