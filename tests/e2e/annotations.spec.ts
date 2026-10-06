// The highlighter and notes (B2-07) in the real app. A word is selected where search finds it (as
// in text.spec.ts), marked with the highlighter, and the document saved as a copy; the copy opens
// again with the mark, which is a standard PDF annotation that says nothing of who made it.
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, quit, test } from "./app";

const t = strings.annotations;

const firstPage = (page: Page) => page.getByRole("img", { name: strings.canvas.page(1) });

/** Searches for `query` and returns where its only hit is on screen. */
async function find(page: Page, query: string) {
  await page.keyboard.press("Control+f");
  const input = page.getByRole("textbox", { name: strings.search.placeholder });
  await input.fill(query);
  await input.press("Enter");
  const hit = page.locator("[data-highlights] polygon:not([data-current])");
  await expect(hit).toHaveCount(1);
  const box = await hit.boundingBox();
  if (!box) throw new Error(`${query} is not on screen`);
  await page.keyboard.press("Escape");
  return box;
}

/** The marks of the highlighter on the first page, as the page lists them (by their outlines). */
const marks = (page: Page) => page.locator("[data-page-annotations] [data-annotation=highlight]");

test("text marked with the highlighter is in the saved copy, as a standard annotation without an author (B2-07)", async ({
  launch,
}) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-annotations-"));
  try {
    const file = path.join(folder, "original.pdf");
    copyFileSync(corpus("benign/mixed-text-zh-en.pdf"), file);
    const page = await launch(file);
    await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
    await expect(marks(page)).toHaveCount(0);

    // Select "Privacy" with a double click, and mark it green from the context menu.
    const privacy = await find(page, "Privacy");
    await page.mouse.dblclick(privacy.x + privacy.width / 2, privacy.y + privacy.height / 2);
    await page.mouse.click(privacy.x + privacy.width / 2, privacy.y + privacy.height / 2, { button: "right" });
    await page.getByRole("menuitem", { name: t.highlight }).click();
    await page.getByRole("menuitem", { name: t.colors.green }).click();
    await expect(marks(page)).toHaveCount(1);
    await expect(marks(page).first()).toHaveAccessibleName(t.highlightIn(t.colors.green));
    // The document has changes now.
    await expect(page.getByRole("tab", { name: new RegExp(`^original\\.pdf\\s*${strings.tabs.unsaved}$`) })).toBeVisible();

    // The mark is where the word is: the outline of the annotation covers what search found.
    const outline = await marks(page).first().boundingBox();
    if (!outline) throw new Error("the mark has no place on the page");
    expect(outline.x).toBeLessThanOrEqual(privacy.x + 1);
    expect(outline.x + outline.width).toBeGreaterThanOrEqual(privacy.x + privacy.width - 1);
    expect(outline.y).toBeLessThanOrEqual(privacy.y + 1);
    expect(outline.y + outline.height).toBeGreaterThanOrEqual(privacy.y + privacy.height - 1);

    // Undo takes it away, redo brings it back; the colour can be changed from its toolbar.
    await page.keyboard.press("Control+z");
    await expect(marks(page)).toHaveCount(0);
    await page.keyboard.press("Control+y");
    await expect(marks(page)).toHaveCount(1);
    await marks(page).first().focus();
    await page.getByRole("toolbar", { name: t.highlightIn(t.colors.green) }).getByRole("button", { name: t.colors.pink }).click();
    await expect(marks(page).first()).toHaveAccessibleName(t.highlightIn(t.colors.pink));

    const copy = path.join(folder, "marked.pdf");
    await page.keyboard.press("Control+Shift+S");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("contentinfo")).toContainText(strings.saving.saved);
    await quit(page);

    // A standard annotation, with where it is and how it looks, and nothing of who made it.
    const saved = readFileSync(copy).toString("latin1");
    expect(saved).toMatch(/\/Subtype\s*\/Highlight/);
    expect(saved).toMatch(/\/QuadPoints/);
    // Not an author (/T, as text or as a UTF-16 hex string), a date made or changed (/CreationDate, /M),
    // nor a unique name (/NM).
    expect(saved).not.toMatch(/\/T\s*[(<]/);
    expect(saved).not.toMatch(/\/M\s*[(<]/);
    expect(saved).not.toMatch(/\/(NM|CreationDate)\b/);

    const reopened = await launch(copy);
    await expect(firstPage(reopened)).toHaveAttribute("data-state", "ready");
    await expect(marks(reopened)).toHaveCount(1);
    await expect(marks(reopened).first()).toHaveAccessibleName(t.highlightIn(t.colors.pink));
    // Removing it, and saving, leaves no mark in the file.
    await marks(reopened).first().focus();
    await reopened.getByRole("toolbar", { name: t.highlightIn(t.colors.pink) }).getByRole("button", { name: t.delete }).click();
    await expect(marks(reopened)).toHaveCount(0);
  } finally {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("a note put on the page says what the user typed, and can be changed (B2-07)", async ({ launch }) => {
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
  await expect(firstPage(page)).toHaveAttribute("data-state", "ready");
  const notes = page.locator("[data-page-annotations] [data-annotation=note]");
  await expect(notes).toHaveCount(0);

  const box = await firstPage(page).boundingBox();
  if (!box) throw new Error("the page is not on screen");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, { button: "right" });
  await page.getByRole("menuitem", { name: new RegExp(t.addNote.replace("…", "")) }).click();
  const dialog = page.getByRole("dialog", { name: t.noteDialog.addTitle });
  await dialog.getByLabel(t.noteDialog.label).fill("Check this\nwith the author");
  await dialog.getByRole("button", { name: t.noteDialog.save }).click();
  await expect(notes).toHaveCount(1);
  await expect(notes.first()).toHaveAccessibleName(t.noteSaying("Check this with the author"));

  await notes.first().focus();
  const toolbar = page.getByRole("toolbar", { name: t.noteSaying("Check this with the author") });
  await toolbar.getByRole("button", { name: t.editNote }).click();
  const edit = page.getByRole("dialog", { name: t.noteDialog.editTitle });
  await edit.getByLabel(t.noteDialog.label).fill("Changed");
  await edit.getByRole("button", { name: t.noteDialog.save }).click();
  await expect(notes.first()).toHaveAccessibleName(t.noteSaying("Changed"));
});
