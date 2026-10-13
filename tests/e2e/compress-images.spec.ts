// A compressed copy (B3-01) in the real app: the pictures of a document encoded again, smaller, in a
// copy written where the user says in the system's save dialog (answered through UI Automation).
// The copy opens like any PDF and shows the picture; the document's own file is not touched.
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { deflateSync } from "node:zlib";

import type { Page } from "@playwright/test";

import { strings } from "../../src/i18n/zh-TW";
import { answerFileDialog, corpus, expect, test } from "./app";

const t = strings.compress;

const sha256 = (file: string) => createHash("sha256").update(readFileSync(file)).digest("hex");

/** A one-page PDF with a photograph on it: smooth colour with noise, 900 x 600, stored as Flate. */
function pdfWithAPhotograph(): Buffer {
  const [width, height] = [900, 600];
  let seed = 0x2545f491;
  const noise = () => {
    seed ^= seed << 13;
    seed ^= seed >>> 17;
    seed ^= seed << 5;
    return ((seed >>> 0) % 25) - 12;
  };
  const pixels = Buffer.alloc(width * height * 3);
  let at = 0;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      for (const base of [(x * 255) / width, (y * 255) / height, ((x + y) * 255) / (width + height)]) {
        pixels[at++] = Math.max(0, Math.min(255, Math.round(base + noise())));
      }
    }
  }
  const image = deflateSync(pixels);
  const content = Buffer.from("q 450 0 0 300 80 400 cm /Im0 Do Q");
  const stream = (dictionary: string, data: Buffer) =>
    Buffer.concat([
      Buffer.from(`<< ${dictionary} /Length ${data.length} >>\nstream\n`),
      data,
      Buffer.from("\nendstream"),
    ]);
  const objects = [
    Buffer.from("<< /Type /Catalog /Pages 2 0 R >>"),
    Buffer.from("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
    Buffer.from(
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>",
    ),
    stream("", content),
    stream(
      `/Type /XObject /Subtype /Image /Width ${width} /Height ${height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode`,
      image,
    ),
  ];
  const parts = [Buffer.from("%PDF-1.7\n")];
  const offsets: number[] = [];
  let length = parts[0]!.length;
  objects.forEach((body, index) => {
    offsets.push(length);
    const piece = Buffer.concat([Buffer.from(`${index + 1} 0 obj\n`), body, Buffer.from("\nendobj\n")]);
    parts.push(piece);
    length += piece.length;
  });
  const table = ["xref", `0 ${objects.length + 1}`, "0000000000 65535 f "].concat(
    offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n `),
  );
  parts.push(
    Buffer.from(
      `${table.join("\n")}\ntrailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${length}\n%%EOF\n`,
    ),
  );
  return Buffer.concat(parts);
}

/** How many pixels of the page as drawn (its canvas) are not close to white. */
const inkOf = (page: Page) =>
  page
    .getByRole("img", { name: strings.canvas.page(1) })
    .last()
    .locator("canvas")
    .evaluate((canvas: HTMLCanvasElement) => {
      const { width, height } = canvas;
      const data = canvas.getContext("2d")!.getImageData(0, 0, width, height).data;
      let count = 0;
      for (let at = 0; at < data.length; at += 4) {
        if (data[at]! + data[at + 1]! + data[at + 2]! < 690) count++;
      }
      return { count, total: width * height };
    });

test.describe("a compressed copy", () => {
  let folder: string;
  test.beforeEach(() => {
    folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-compress-"));
  });
  test.afterEach(() => {
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  });

  test("has the picture encoded again, smaller, opens like any PDF, and the document's file is untouched", async ({
    launch,
  }) => {
    const original = path.join(folder, "photo.pdf");
    writeFileSync(original, pdfWithAPhotograph());
    const before = sha256(original);
    const page = await launch(original);
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );

    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(strings.menu.compressImages) }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await expect(dialog).toContainText(t.signatures);
    // What it starts with, and the lowest quality for the smallest copy.
    await expect(dialog.getByRole("radio", { name: t.quality.medium })).toBeChecked();
    await expect(dialog.getByRole("radio", { name: t.size(2000) })).toBeChecked();
    await dialog.getByRole("radio", { name: t.quality.low }).click();
    await dialog.getByRole("button", { name: t.start }).click();

    const copy = path.join(folder, "photo-small.pdf");
    const report = await answerFileDialog(page, { path: copy });
    // What came of it stays until it has been read: one picture, encoded again.
    const done = page.getByRole("dialog", { name: t.resultTitle });
    await expect(done).toContainText("1 張圖片中的 1 張重新編碼了");
    expect(readdirSync(folder).sort(), report).toEqual(["photo-small.pdf", "photo.pdf"]);
    const bytes = readFileSync(copy);
    expect(bytes.subarray(0, 5).toString()).toBe("%PDF-");
    expect(bytes.includes(Buffer.from("/DCTDecode"))).toBe(true);
    expect(bytes.length * 4).toBeLessThan(readFileSync(original).length);
    // The document's own file, and its tab, are as they were.
    expect(sha256(original)).toBe(before);
    await done.getByRole("button", { name: t.close }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByRole("tab", { name: /photo\.pdf/ })).toHaveAttribute("aria-selected", "true");

    // The copy opens like any PDF, in a tab of its own, and shows the picture.
    await page.keyboard.press("Control+o");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("tab", { name: /photo-small\.pdf/ })).toHaveAttribute("aria-selected", "true");
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).last()).toHaveAttribute(
      "data-state",
      "ready",
    );
    // The picture is 450 x 300 points of a 612 x 792 page: about a quarter of it.
    await expect
      .poll(async () => {
        const { count, total } = await inkOf(page);
        return count / total;
      })
      .toBeGreaterThan(0.15);
  });

  test("says so when the document has no picture, and writes the copy all the same", async ({ launch }) => {
    const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await page.getByRole("button", { name: strings.toolbar.more }).click();
    await page.getByRole("menuitem", { name: new RegExp(strings.menu.compressImages) }).click();
    const dialog = page.getByRole("dialog", { name: t.title });
    await dialog.getByRole("button", { name: t.start }).click();
    const copy = path.join(folder, "text-only.pdf");
    await answerFileDialog(page, { path: copy });
    await expect(page.getByRole("dialog", { name: t.resultTitle })).toContainText("文件裡沒有可以壓縮的圖片");
    expect(readFileSync(copy).subarray(0, 5).toString()).toBe("%PDF-");
  });
});
