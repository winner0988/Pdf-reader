// A batch (B3-04) in the real app: the menu's dialog, the system's open dialog for several files
// and the folder dialog (both answered through UI Automation), each file done in a worker of its
// own, and a copy of each in the folder; no file is changed, and one that fails stops no other.
import { createHash } from "node:crypto";
import { copyFileSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.batch;
const e = strings.encryption;
/** The author the metadata sample names everywhere (tests/corpus/generate.py). */
const AUTHOR = Buffer.from("Jane Q. Private-Author");

const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

test.describe("a batch", () => {
  let sources: string;
  let out: string;
  test.beforeEach(() => {
    sources = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-batch-in-"));
    out = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-batch-out-"));
  });
  test.afterEach(() => {
    rmSync(sources, { recursive: true, force: true });
    rmSync(out, { recursive: true, force: true });
  });

  /** Copies corpus files into the sources folder under new names; returns their paths. */
  function files(pairs: [string, string][]): string[] {
    return pairs.map(([sample, name]) => {
      const file = path.join(sources, name);
      copyFileSync(corpus(sample), file);
      return file;
    });
  }

  async function openDialog(page: import("@playwright/test").Page) {
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(`^${strings.menu.batch}`) }).click();
    return page.getByRole("dialog", { name: t.title });
  }

  test("the metadata of each file goes, a file with a password is passed over, and no file is changed", async ({
    launch,
  }) => {
    const page = await launch();
    const picked = files([
      ["benign/metadata-full.pdf", "a.pdf"],
      ["benign/encrypted-aes256.pdf", "b.pdf"],
      ["benign/single-page.pdf", "c.pdf"],
    ]);
    const before = picked.map(sha256);
    const dialog = await openDialog(page);
    await dialog.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: picked.map((file) => `"${file}"`).join(" ") });
    await answerFileDialog(page, { path: out });

    await expect(dialog.getByRole("status")).toHaveText(t.summary(2, 1, 0));
    const list = dialog.getByRole("list", { name: t.filesLabel });
    await expect(list).toContainText(t.done("a（隱私匯出）.pdf"));
    await expect(list).toContainText(t.failedFile(t.reasons.encrypted!));
    expect(readdirSync(out).sort()).toEqual(["a（隱私匯出）.pdf", "c（隱私匯出）.pdf"]);
    const copy = readFileSync(path.join(out, "a（隱私匯出）.pdf"));
    expect(copy.subarray(0, 5).toString()).toBe("%PDF-");
    expect(copy.includes(AUTHOR)).toBe(false);
    expect(picked.map(sha256)).toEqual(before);

    // Done again into the same folder: the copies there stay, the new ones get a number.
    await dialog.getByRole("button", { name: t.close }).click();
    const again = await openDialog(page);
    await again.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: picked[0]! });
    await answerFileDialog(page, { path: out });
    await expect(again.getByRole("status")).toHaveText(t.summary(1, 0, 0));
    expect(readdirSync(out).sort()).toEqual([
      "a（隱私匯出） (2).pdf",
      "a（隱私匯出）.pdf",
      "c（隱私匯出）.pdf",
    ]);
  });

  test("what a camera wrote into the pictures goes, and a signed file is left alone", async ({ launch }) => {
    const page = await launch();
    const picked = files([
      ["benign/photo-exif.pdf", "photo.pdf"],
      ["benign/signed.pdf", "signed.pdf"],
    ]);
    const before = picked.map(sha256);
    const dialog = await openDialog(page);
    await dialog.getByRole("radio", { name: t.removeExif }).check();
    await expect(dialog).toContainText(t.exifNote);
    await dialog.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: picked.map((file) => `"${file}"`).join(" ") });
    await answerFileDialog(page, { path: out });

    await expect(dialog.getByRole("status")).toHaveText(t.summary(1, 1, 0));
    const list = dialog.getByRole("list", { name: t.filesLabel });
    await expect(list).toContainText(t.done("photo（已清除 EXIF）.pdf"));
    await expect(list).toContainText(t.failedFile(t.reasons.invalidArgument!));
    expect(readdirSync(out)).toEqual(["photo（已清除 EXIF）.pdf"]);
    const copy = readFileSync(path.join(out, "photo（已清除 EXIF）.pdf"));
    expect(copy.subarray(0, 5).toString()).toBe("%PDF-");
    // The camera (EXIF), the author (XMP, IPTC, the comment) and the thumbnail are gone.
    for (const gone of ["Canon", "Exif", "Uncropped thumbnail data", "Photoshop 3.0"]) {
      expect(copy.includes(Buffer.from(gone)), gone).toBe(false);
    }
    expect(copy.includes(AUTHOR)).toBe(false);
    expect(picked.map(sha256)).toEqual(before);
  });

  test("each file is encrypted with the same password, and the dialog keeps none of it", async ({ launch }) => {
    const page = await launch();
    const picked = files([
      ["benign/single-page.pdf", "one.pdf"],
      ["benign/multi-page-10.pdf", "two.pdf"],
    ]);
    let dialog = await openDialog(page);
    await dialog.getByRole("radio", { name: t.encrypt }).check();
    await dialog.getByLabel(e.openPassword, { exact: true }).fill("pw-batch-7Qz");
    await dialog.getByLabel(`${e.openPassword}（${e.again}）`).fill("pw-batch-7Qz");
    await dialog.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: picked.map((file) => `"${file}"`).join(" ") });
    await answerFileDialog(page, { path: out });

    await expect(dialog.getByRole("status")).toHaveText(t.summary(2, 0, 0));
    expect(readdirSync(out).sort()).toEqual(["one（已加密）.pdf", "two（已加密）.pdf"]);
    for (const name of readdirSync(out)) {
      const bytes = readFileSync(path.join(out, name));
      expect(bytes.includes(Buffer.from("/Encrypt")), name).toBe(true);
      expect(bytes.includes(Buffer.from("pw-batch-7Qz")), name).toBe(false);
    }
    await dialog.getByRole("button", { name: t.close }).click();
    dialog = await openDialog(page);
    await dialog.getByRole("radio", { name: t.encrypt }).check();
    await expect(dialog.getByLabel(e.openPassword, { exact: true })).toHaveValue("");
  });
});
