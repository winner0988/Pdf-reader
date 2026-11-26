import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  ICO_SIZES,
  mergeIco,
  PDF_FILE_PIXEL_SIZES,
  PDF_FILE_SIZES,
  pngPicture,
  readIco,
  replacePictures,
  SMALL_SIZES,
  writeIco,
} from "./app-icons.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

/** A picture of `size` px whose data is `text`. */
function picture(size, text) {
  const head = Buffer.alloc(8);
  head[0] = size === 256 ? 0 : size;
  head[1] = head[0];
  head.writeUInt16LE(1, 4);
  head.writeUInt16LE(32, 6);
  return { size, head, data: Buffer.from(text) };
}

const ico = (name, sizes = ICO_SIZES) => writeIco(sizes.map((size) => picture(size, `${name}-${size}`)));
const told = (bytes) => readIco(bytes).map((entry) => entry.data.toString());

test("an .ico file reads back as it was written", () => {
  const pictures = [picture(16, "small"), picture(256, "a much longer picture")];
  assert.deepEqual(readIco(writeIco(pictures)), pictures);
});

test("256 px is written as a width of 0", () => {
  const bytes = writeIco([picture(256, "x")]);
  assert.equal(bytes[6], 0);
  assert.equal(readIco(bytes)[0].size, 256);
});

test("the small sizes come from the simplified icon, the others from the full one", () => {
  assert.deepEqual(told(mergeIco(ico("full"), ico("small"))), [
    "small-16",
    "small-24",
    "small-32",
    "full-48",
    "full-64",
    "full-256",
  ]);
});

test("the merged file keeps the full icon's order", () => {
  const merged = mergeIco(ico("full", [256, 32, 16]), ico("small"));
  assert.deepEqual(told(merged), ["full-256", "small-32", "small-16"]);
});

test("a small size the simplified icon lacks is an error", () => {
  assert.throws(() => mergeIco(ico("full"), ico("small", [16, 32])), /no 24 px picture/);
});

test("what is not an .ico file is refused", () => {
  assert.throws(() => readIco(Buffer.from("PNG")), /not an \.ico file/);
  assert.throws(() => readIco(Buffer.from([0, 0, 2, 0, 1, 0])), /not an \.ico file/);
});

test("a file cut short is refused", () => {
  const bytes = ico("full");
  assert.throws(() => readIco(bytes.subarray(0, 20)), /directory is cut short/);
  assert.throws(() => readIco(bytes.subarray(0, bytes.length - 1)), /picture is cut short/);
});

/** The first bytes of a PNG file of `width` by `height` px: its signature and its image header. */
function png(width, height) {
  const bytes = Buffer.alloc(33);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(bytes);
  bytes.writeUInt32BE(13, 8);
  bytes.write("IHDR", 12, "latin1");
  bytes.writeUInt32BE(width, 16);
  bytes.writeUInt32BE(height, 20);
  return bytes;
}

test("a PNG becomes an .ico picture of its size, with the entry the Tauri CLI writes", () => {
  const twenty = pngPicture(20, png(20, 20));
  assert.equal(twenty.size, 20);
  assert.deepEqual([...twenty.head], [20, 20, 0, 0, 0, 0, 32, 0]);
  assert.deepEqual(twenty.data, png(20, 20));
  assert.deepEqual(readIco(writeIco([twenty])), [twenty]);
  assert.deepEqual([...pngPicture(256, png(256, 256)).head], [0, 0, 0, 0, 0, 0, 32, 0]);
});

test("a PNG of another size, or what is not a PNG, is refused", () => {
  assert.throws(() => pngPicture(20, png(24, 24)), /not 20 by 20 px/);
  assert.throws(() => pngPicture(20, png(20, 24)), /not 20 by 20 px/);
  assert.throws(() => pngPicture(20, Buffer.from("GIF89a, not a PNG file at all")), /not a PNG file/);
  assert.throws(() => pngPicture(20, png(20, 20).subarray(0, 23)), /not a PNG file/);
});

test("pictures take the place of those of their size, or are added, smallest first", () => {
  const merged = replacePictures(ico("full"), [picture(32, "own-32"), picture(20, "own-20"), picture(16, "own-16")]);
  assert.deepEqual(told(merged), ["own-16", "own-20", "full-24", "own-32", "full-48", "full-64", "full-256"]);
});

/** The .ico files the exe holds and the sizes each must have: the app icon, the dark tile, and the PDF file icon. */
const ICONS = new Map([
  ["icon.ico", ICO_SIZES],
  ["icon-dark.ico", ICO_SIZES],
  ["pdf-file.ico", PDF_FILE_SIZES],
]);

for (const [name, sizes] of ICONS) {
  test(`the app's ${name} has every size Windows asks for`, () => {
    const pictures = readIco(readFileSync(path.join(ROOT, "src-tauri", "icons", name)));
    assert.deepEqual(pictures.map((entry) => entry.size).sort((a, b) => a - b), sizes);
    for (const entry of pictures) assert.ok(entry.data.length > 0, `${entry.size} px is empty`);
  });
}

test("the icons are different pictures at every size", () => {
  const read = (name) => {
    const pictures = readIco(readFileSync(path.join(ROOT, "src-tauri", "icons", name)));
    return new Map(pictures.map((entry) => [entry.size, entry.data]));
  };
  const icons = [...ICONS.keys()].map((name) => [name, read(name)]);
  for (const [index, [first, pictures]] of icons.entries()) {
    for (const [second, others] of icons.slice(index + 1)) {
      for (const [size, data] of pictures) {
        if (others.has(size)) assert.ok(!data.equals(others.get(size)), `${first} and ${second}, ${size} px`);
      }
    }
  }
});

test("each small size of the PDF file icon has a source drawn on its own pixel grid", () => {
  for (const size of PDF_FILE_PIXEL_SIZES) {
    const source = readFileSync(path.join(ROOT, "src-tauri", "icons", `pdf-file-${size}.svg`), "utf8");
    assert.ok(source.includes(`viewBox="0 0 ${size} ${size}"`), `pdf-file-${size}.svg is not drawn on a ${size} px grid`);
  }
});

test("the PDF file icon has every size the app icon has, and each size with its own source", () => {
  for (const size of [...ICO_SIZES, ...PDF_FILE_PIXEL_SIZES]) assert.ok(PDF_FILE_SIZES.includes(size), `${size} px`);
});

test("every simplified size is one icon.ico has", () => {
  for (const size of SMALL_SIZES) assert.ok(ICO_SIZES.includes(size));
});
