// The annotation summary (B3-18) in the real app. Two words are marked with the highlighter, the
// one further right first; the summary lists them as the page is read, with the words they cover,
// in the file chosen in the system's save dialog (answered here through UI Automation).
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.exportAnnotations;

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

/** Selects the word `query` with a double click and marks it with the highlighter. */
async function highlight(page: Page, query: string) {
  const word = await find(page, query);
  const middle = { x: word.x + word.width / 2, y: word.y + word.height / 2 };
  await page.mouse.dblclick(middle.x, middle.y);
  await page.mouse.click(middle.x, middle.y, { button: "right" });
  await page.getByRole("menuitem", { name: strings.annotations.highlight }).click();
  await page.getByRole("menuitem", { name: strings.annotations.colors.yellow }).click();
}

const marks = (page: Page) => page.locator("[data-page-annotations] [data-annotation=highlight]");

test.describe("annotation summary", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-summary-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("lists the marks as the page is read, with the words they cover (B3-18)", async ({ launch }) => {
    const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
    await highlight(page, "Reader");
    await expect(marks(page)).toHaveCount(1);
    await highlight(page, "Privacy");
    await expect(marks(page)).toHaveCount(2);

    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.exportAnnotations}`) }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await dialog.getByRole("radio", { name: t.markdown }).check();
    await dialog.getByRole("button", { name: t.start }).click();

    const file = path.join(folder, "summary.md");
    const dialogReport = await answerFileDialog(page, { path: file });
    await expect(page.getByRole("contentinfo")).toContainText(t.done(2));
    expect(existsSync(file), `${dialogReport}\nthe folder holds: ${readdirSync(folder).join(", ")}`).toBe(true);
    const lines = readFileSync(file, "utf8").split("\r\n");
    expect(lines.slice(0, 4)).toEqual(["# mixed-text-zh-en.pdf", "", "## 第 1 頁", ""]);
    // Read from the left, though "Reader" was marked first.
    expect(lines[4]).toMatch(/^- \*\*螢光筆標示\*\*：Privacy/);
    expect(lines[5]).toBe("- **螢光筆標示**：Reader");
    expect(lines.slice(6)).toEqual([""]);
    // Written whole: no temporary file is left behind.
    expect(readdirSync(folder)).toEqual(["summary.md"]);
  });

  test("says so when the document has no annotations, and writes nothing", async ({ launch }) => {
    const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.exportAnnotations}`) }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await dialog.getByRole("button", { name: t.start }).click();
    await expect(dialog.getByRole("status")).toHaveText(t.empty);
    expect(readdirSync(folder)).toEqual([]);
  });
});
