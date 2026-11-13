import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { ICO_SIZES, mergeIco, readIco, SMALL_SIZES, writeIco } from "./app-icons.mjs";

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

test("the app's icon.ico has every size Windows asks for", () => {
  const pictures = readIco(readFileSync(path.join(ROOT, "src-tauri", "icons", "icon.ico")));
  assert.deepEqual(pictures.map((entry) => entry.size).sort((a, b) => a - b), ICO_SIZES);
  for (const entry of pictures) assert.ok(entry.data.length > 0, `${entry.size} px is empty`);
});

test("every simplified size is one icon.ico has", () => {
  for (const size of SMALL_SIZES) assert.ok(ICO_SIZES.includes(size));
});
