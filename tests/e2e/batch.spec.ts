// A batch (B3-04) in the real app: the menu's dialog, the system's open dialog for several files
// and the folder dialog (both answered through UI Automation), each file done in a worker of its
// own, and a copy of each in the folder; no file is changed, and one that fails stops no other.
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import {
  answerFileDialog,
  appRunning,
  appWindowShown,
  closeAppWindow,
  closeMessageBox,
  corpus,
  expect,
  launchAgain,
  test,
} from "./app";

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

  /** Starts a batch that removes the metadata of `picked` into `out`; returns the dialog. */
  async function startBatch(page: import("@playwright/test").Page, picked: string[]) {
    const dialog = await openDialog(page);
    await dialog.getByRole("button", { name: t.start }).click();
    await answerFileDialog(page, { path: picked.map((file) => `"${file}"`).join(" ") });
    await answerFileDialog(page, { path: out });
    await expect(dialog.getByRole("list", { name: t.filesLabel })).toBeVisible();
    return dialog;
  }

  /**
   * A PDF of one page that takes a worker a while to copy: two million numbers it reads and
   * writes again. A copy of a sample from the corpus takes some 50 ms, and a batch of ten of them
   * would be done before the window is closed.
   */
  function heavyPdf(): Buffer {
    const objects = [
      "<< /Type /Catalog /Pages 2 0 R /Numbers 4 0 R >>",
      "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
      `[${"0 ".repeat(2_000_000)}]`,
    ];
    let pdf = "%PDF-1.7\n";
    const offsets = objects.map((body, index) => {
      const offset = pdf.length;
      pdf += `${index + 1} 0 obj\n${body}\nendobj\n`;
      return offset;
    });
    const xref = pdf.length;
    pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
    for (const offset of offsets) pdf += `${String(offset).padStart(10, "0")} 00000 n \n`;
    pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
    return Buffer.from(pdf, "latin1");
  }

  /** Ten files: nine of them heavy, and the last with a password. */
  function tenFiles(): string[] {
    const heavy = heavyPdf();
    const picked = Array.from({ length: 9 }, (_, index) => {
      const file = path.join(sources, `f${index + 1}.pdf`);
      writeFileSync(file, heavy);
      return file;
    });
    return [...picked, ...files([["benign/encrypted-aes256.pdf", "locked.pdf"]])];
  }

  test("with the window closed the batch goes on, reports each file in its folder, and the app ends", async ({
    launch,
  }) => {
    const page = await launch();
    const picked = tenFiles();
    await startBatch(page, picked);
    expect(await appWindowShown(page)).toBe(true);
    // Closed while the batch runs: the window goes, the app does not.
    await closeAppWindow(page);
    await expect.poll(() => appWindowShown(page)).toBe(false);
    expect(appRunning(page)).toBe(true);
    // Done: the report is in the folder, a message says so (src-tauri/src/strings.rs), and once
    // it is read the app ends. The batch takes what it takes: the message is looked for once the
    // report is there.
    const reportFile = path.join(out, "批次處理報告.txt");
    await expect.poll(() => existsSync(reportFile), { timeout: 120_000 }).toBe(true);
    expect(await closeMessageBox(page)).toContain("批次處理完成");
    await expect.poll(() => appRunning(page), { timeout: 15_000 }).toBe(false);
    const names = readdirSync(out);
    expect(names.filter((name) => name.endsWith("（隱私匯出）.pdf"))).toHaveLength(9);
    const report = readFileSync(reportFile, "utf8");
    expect(report).toContain("完成 9 個，失敗 1 個。");
    // Lines Notepad shows as lines.
    expect(report).toContain("locked.pdf\r\n  失敗：有密碼的檔案");
    expect(report).toContain("f1.pdf\r\n  完成：f1（隱私匯出）.pdf");
  });

  test("showing the window again calls the close off: the batch ends in the dialog and the app stays", async ({
    launch,
  }) => {
    const page = await launch();
    const picked = tenFiles();
    const dialog = await startBatch(page, picked);
    expect(await appWindowShown(page)).toBe(true);
    await closeAppWindow(page);
    await expect.poll(() => appWindowShown(page)).toBe(false);
    // A second launch hands the window a file, and shows it again.
    launchAgain(corpus("benign/single-page.pdf"));
    await expect.poll(() => appWindowShown(page)).toBe(true);
    await expect(dialog.getByRole("status")).toHaveText(t.summary(9, 1, 0), { timeout: 60_000 });
    expect(appRunning(page)).toBe(true);
    expect(readdirSync(out)).not.toContain("批次處理報告.txt");
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
