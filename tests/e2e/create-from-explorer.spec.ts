// Creating a PDF from images passed through File Explorer's context menu (`--create-pdf-from`,
// B5-13, #450): multiple separate launches are collected by the running instance into one batch,
// sorted by file name, saved via the shared save dialog, and opened as a new PDF tab. Unsupported
// files are skipped with a native explanation before saving (or without showing a save dialog if
// every file is skipped).
import { copyFileSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, closeMessageBox, corpus, documentTabs, expect, launchAgainWithArgs, test } from "./app";

const t = strings.create;

test.describe("making a PDF from Explorer context menu (--create-pdf-from)", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-explorer-create-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true });
  });

  test("three separate --create-pdf-from launches are collected into one three-page PDF tab", async ({ launch }) => {
    const page = await launch();

    const img1 = path.join(folder, "1-first.jpg");
    const img2 = path.join(folder, "2-second.png");
    const img3 = path.join(folder, "10-third.jpg");
    copyFileSync(corpus("images/stamp-exif.jpg"), img1);
    copyFileSync(corpus("images/stamp-metadata.png"), img2);
    copyFileSync(corpus("images/stamp-exif.jpg"), img3);

    const startedAt = Date.now();
    launchAgainWithArgs(["--create-pdf-from", img3]);
    launchAgainWithArgs(["--create-pdf-from", img1]);
    launchAgainWithArgs(["--create-pdf-from", img2]);
    const elapsedMs = Date.now() - startedAt;
    console.log(`--- explorer-create: 3 launches took ${elapsedMs} ms ---`);

    // The collector waits 1.5 s after the last arrival, then shows the save dialog.
    const made = path.join(folder, "from-explorer.pdf");
    const report = await answerFileDialog(page, { path: made });

    await expect(page.getByRole("tab", { name: /from-explorer\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(3) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await expect(page.getByRole("img", { name: strings.canvas.page(4) })).toHaveCount(0);
    await expect(documentTabs(page)).toHaveCount(1);

    const bytes = readFileSync(made);
    expect(bytes.subarray(0, 5).toString(), report).toBe("%PDF-");
    for (const private_ of ["Canon", "2023:07:04", "Exif", "Private-Author"]) {
      expect(bytes.includes(Buffer.from(private_)), private_).toBe(false);
    }
    expect(readdirSync(folder).sort()).toEqual(["1-first.jpg", "10-third.jpg", "2-second.png", "from-explorer.pdf"]);
  });

  test("an unsupported file is skipped with an explanation while valid images still become a PDF", async ({ launch }) => {
    const page = await launch();

    const img1 = path.join(folder, "a.jpg");
    const img2 = path.join(folder, "b.png");
    const unsupported = path.join(folder, "notes.txt");
    copyFileSync(corpus("images/stamp-exif.jpg"), img1);
    copyFileSync(corpus("images/stamp-metadata.png"), img2);
    writeFileSync(unsupported, "not an image");

    launchAgainWithArgs(["--create-pdf-from", img1]);
    launchAgainWithArgs(["--create-pdf-from", unsupported]);
    launchAgainWithArgs(["--create-pdf-from", img2]);

    // Before the save dialog, a message box explains how many files were skipped and why.
    const skippedBox = await closeMessageBox(page);
    expect(skippedBox).toContain("部分檔案已略過");

    const made = path.join(folder, "partial.pdf");
    await answerFileDialog(page, { path: made });

    await expect(page.getByRole("tab", { name: /partial\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(2) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await expect(page.getByRole("img", { name: strings.canvas.page(3) })).toHaveCount(0);
    await expect(page.getByRole("status").filter({ hasText: t.explorerDoneSkipped(2, 1) })).toBeVisible();
  });

  test("when every file is unsupported, a notice explains why and no save dialog opens", async ({ launch }) => {
    const page = await launch();

    const unsupported = path.join(folder, "only.txt");
    writeFileSync(unsupported, "plain text");

    launchAgainWithArgs(["--create-pdf-from", unsupported]);

    const box = await closeMessageBox(page);
    expect(box).toContain("無法建立 PDF");
    await expect(documentTabs(page)).toHaveCount(0);
    expect(readdirSync(folder)).toEqual(["only.txt"]);
  });
});
