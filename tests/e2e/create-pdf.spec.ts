// Making a PDF of files (B3-03) in the real app: the start screen's button, the system's open
// dialog for the files and the save dialog for the PDF (both answered through UI Automation), a
// sandboxed worker that reads the files, and the new PDF opening in a tab. Hostile files end in a
// sentence, not in the end of the app.
import { copyFileSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, documentTabs, expect, test } from "./app";

const t = strings.create;

test.describe("making a PDF of files", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-create-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  /** Opens the dialog from the start screen and presses its button. */
  async function start(page: import("@playwright/test").Page) {
    await page.getByRole("button", { name: strings.empty.createButton }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toContainText(t.experimental);
    await dialog.getByRole("button", { name: t.start }).click();
    return dialog;
  }

  test("a Word file becomes a PDF that opens in a tab of its own, with its words and without its author", async ({
    launch,
  }) => {
    const page = await launch();
    await start(page);
    await answerFileDialog(page, { path: corpus("sources/letter.docx") });
    const made = path.join(folder, "letter.pdf");
    const report = await answerFileDialog(page, { path: made });

    await expect(page.getByRole("tab", { name: /letter\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    // Only the PDF is in the folder: nothing was left behind.
    expect(readdirSync(folder), report).toEqual(["letter.pdf"]);
    const bytes = readFileSync(made);
    expect(bytes.subarray(0, 5).toString()).toBe("%PDF-");
    for (const private_ of ["Private-Author", "Private title", "/Info", "/Author"]) {
      expect(bytes.includes(Buffer.from(private_)), private_).toBe(false);
    }
    // The dialog is gone; the original is where it was.
    await expect(page.getByRole("dialog")).toHaveCount(0);
    expect(readFileSync(corpus("sources/letter.docx")).length).toBeGreaterThan(1000);
  });

  test("pictures go together, a page each, and bring nothing a camera wrote", async ({ launch }) => {
    const page = await launch();
    // Both in a folder of their own, so that the dialog takes the two names typed as two files.
    const first = path.join(folder, "1-red.jpg");
    const second = path.join(folder, "2-blue.png");
    copyFileSync(corpus("images/stamp-exif.jpg"), first);
    copyFileSync(corpus("images/stamp-metadata.png"), second);
    await start(page);
    await answerFileDialog(page, { path: `"${second}" "${first}"` });
    const made = path.join(folder, "pictures.pdf");
    await answerFileDialog(page, { path: made });

    await expect(page.getByRole("tab", { name: /pictures\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(2) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await expect(page.getByRole("img", { name: strings.canvas.page(3) })).toHaveCount(0);
    const bytes = readFileSync(made);
    for (const private_ of ["Canon", "2023:07:04", "Exif", "Private-Author"]) {
      expect(bytes.includes(Buffer.from(private_)), private_).toBe(false);
    }
  });

  test("a web page that points at remote files and has a script becomes a PDF of its words", async ({ launch }) => {
    const page = await launch();
    await start(page);
    await answerFileDialog(page, { path: corpus("sources/web-page.html") });
    await answerFileDialog(page, { path: path.join(folder, "web-page.pdf") });
    await expect(page.getByRole("tab", { name: /web-page\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    const bytes = readFileSync(path.join(folder, "web-page.pdf"));
    for (const remote of ["beacon.example.invalid", "SCRIPT RAN", "win.ini"]) {
      expect(bytes.includes(Buffer.from(remote)), remote).toBe(false);
    }
  });

  test("closing a system dialog leaves the dialog for another try, and nothing is made", async ({ launch }) => {
    const page = await launch();
    const dialog = await start(page);
    await answerFileDialog(page, "cancel");
    await expect(dialog.getByRole("button", { name: t.start })).toBeEnabled();
    await expect(dialog.getByRole("alert")).toHaveCount(0);

    // The files chosen, and the place not.
    await dialog.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: corpus("sources/text.txt") });
    await answerFileDialog(page, "cancel");
    await expect(dialog.getByRole("button", { name: t.start })).toBeEnabled();
    expect(readdirSync(folder)).toEqual([]);
    await expect(documentTabs(page)).toHaveCount(0);
  });

  test("files that are no PDF's sources end in a sentence, and the app goes on", async ({ launch }) => {
    const page = await launch();
    const dialog = await start(page);

    // A file that is not what its name says.
    const broken = path.join(folder, "broken.docx");
    writeFileSync(broken, "not a zip at all");
    await answerFileDialog(page, { path: broken });
    await answerFileDialog(page, { path: path.join(folder, "broken.pdf") });
    await expect(dialog.getByRole("alert")).toHaveText(t.failures.corrupted!);

    // A Word file that says it is 3.5 GiB unpacked, in two kilobytes.
    await dialog.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: corpus("sources/bomb-declared-size.docx") });
    await answerFileDialog(page, { path: path.join(folder, "bomb.pdf") });
    await expect(dialog.getByRole("alert")).toHaveText(t.failures.corrupted!);
    expect(readdirSync(folder).sort()).toEqual(["broken.docx"]);

    // The app works: the next one is made.
    await dialog.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: corpus("sources/book.epub") });
    await answerFileDialog(page, { path: path.join(folder, "book.pdf") });
    await expect(page.getByRole("tab", { name: /book\.pdf/ })).toHaveAttribute("aria-selected", "true");
    expect(readdirSync(folder).sort()).toEqual(["book.pdf", "broken.docx"]);
  });

  test("is in the menu of an open document, and the PDF opens next to it", async ({ launch }) => {
    const page = await launch(corpus("benign/single-page.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.createPdf}`) }).click();
    await page.getByRole("dialog", { name: t.title }).getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: corpus("sources/sheet.xlsx") });
    await answerFileDialog(page, { path: path.join(folder, "sheet.pdf") });

    await expect(page.getByRole("tab", { name: /sheet\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("tab", { name: /single-page\.pdf/ })).toHaveAttribute("aria-selected", "false");
    await expect(page.getByRole("contentinfo").last()).toBeVisible();
  });
});
