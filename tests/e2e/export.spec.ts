// Exporting (B2-04) in the real app. The page only says what to export; the main process shows
// the system's save or folder dialog (answered here through UI Automation) and writes the files.
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.export;

async function openExport(page: Page) {
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");
  await page.getByRole("button", { name: strings.toolbar.more }).click();
  await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.export}`) }).click();
  return page.getByRole("dialog", { name: t.title });
}

test.describe("export", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-export-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("the text goes to the file chosen in the save dialog", async ({ launch }) => {
    const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
    const dialog = await openExport(page);
    await dialog.getByRole("button", { name: t.start }).click();

    const file = path.join(folder, "exported.txt");
    const dialogReport = await answerFileDialog(page, { path: file });
    await expect(page.getByRole("contentinfo")).toContainText(t.done(1));
    // What the dialog showed and what the folder holds, should the file be elsewhere.
    expect(existsSync(file), `${dialogReport}\nthe folder holds: ${readdirSync(folder).join(", ")}`).toBe(true);
    const text = readFileSync(file, "utf8");
    expect(text).toContain("Privacy-first PDF Reader");
    expect(text).toContain("隱私優先的 PDF 閱讀器");
    expect(text.endsWith("\f")).toBe(true);
    // Written whole: no temporary file is left behind.
    expect(readdirSync(folder)).toEqual(["exported.txt"]);
  });

  test("page images go to the chosen folder, one PNG per page at the chosen resolution", async ({ launch }) => {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    const dialog = await openExport(page);
    await dialog.getByRole("radio", { name: t.png }).check();
    await dialog.getByRole("combobox", { name: t.resolution }).selectOption("72");
    await dialog.getByRole("textbox", { name: strings.print.pages }).fill("2-3");
    await dialog.getByRole("button", { name: t.start }).click();

    await answerFileDialog(page, { path: folder });
    await expect(page.getByRole("contentinfo")).toContainText(t.done(2));
    for (const pageNumber of [2, 3]) {
      const png = readFileSync(path.join(folder, `multi-page-10-p${pageNumber}.png`));
      expect([...png.subarray(0, 8)]).toEqual([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
      // IHDR width: a Letter page is 612 points, 612 pixels at 72 dpi.
      expect(png.readUInt32BE(16)).toBe(612);
    }
    // Only the chosen pages, and no temporary files.
    expect(readdirSync(folder).sort()).toEqual(["multi-page-10-p2.png", "multi-page-10-p3.png"]);
  });

  test("nothing is written when the save dialog is cancelled", async ({ launch }) => {
    const page = await launch(corpus("benign/single-page.pdf"));
    const dialog = await openExport(page);
    await dialog.getByRole("button", { name: t.start }).click();

    await answerFileDialog(page, "cancel");
    // The export dialog stays for another try, and nothing was exported.
    await expect(dialog.getByRole("button", { name: t.start })).toBeEnabled();
    await expect(dialog.getByRole("status")).toHaveCount(0);
  });
});
