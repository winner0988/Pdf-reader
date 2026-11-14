import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { deflateSync } from "node:zlib";

import {
  bmpSize,
  cropOnWhite,
  decodePng,
  encodeBmp,
  IMAGES,
  SCALE,
  soften,
  squared,
  viewBoxSize,
} from "./installer-images.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const WINDOWS = path.join(ROOT, "src-tauri", "windows");

function chunk(type, body) {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(body.length, 0);
  head.write(type, 4, "latin1");
  // The reader does not check the CRC, so it is left 0.
  return Buffer.concat([head, body, Buffer.alloc(4)]);
}

function paeth(left, up, upLeft) {
  const estimate = left + up - upLeft;
  const [toLeft, toUp, toUpLeft] = [left, up, upLeft].map((value) => Math.abs(estimate - value));
  if (toLeft <= toUp && toLeft <= toUpLeft) return left;
  return toUp <= toUpLeft ? up : upLeft;
}

/** A PNG of `pixels` (`channels` bytes each), row `y` stored with filter `filters[y]`. */
function png({ width, height, channels, pixels, filters, depth = 8, interlace = 0 }) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = depth;
  header[9] = channels === 4 ? 6 : 2;
  header[12] = interlace;
  const stride = width * channels;
  const raw = Buffer.alloc(height * (stride + 1));
  for (let y = 0; y < height; y += 1) {
    const filter = filters[y % filters.length];
    raw[y * (stride + 1)] = filter;
    for (let x = 0; x < stride; x += 1) {
      const left = x >= channels ? pixels[y * stride + x - channels] : 0;
      const up = y > 0 ? pixels[(y - 1) * stride + x] : 0;
      const upLeft = y > 0 && x >= channels ? pixels[(y - 1) * stride + x - channels] : 0;
      const predicted = [0, left, up, (left + up) >> 1, paeth(left, up, upLeft)][filter];
      raw[y * (stride + 1) + 1 + x] = (pixels[y * stride + x] - predicted) & 0xff;
    }
  }
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/** Bytes that change from pixel to pixel and row to row, so every filter has work to do. */
function noise(length) {
  return Buffer.from(Array.from({ length }, (_, index) => (index * 97 + (index >> 3) * 31) & 0xff));
}

test("an RGBA PNG reads back with each of the five row filters", () => {
  const [width, height] = [7, 10];
  const pixels = noise(width * height * 4);
  const image = decodePng(png({ width, height, channels: 4, pixels, filters: [0, 1, 2, 3, 4] }));
  assert.deepEqual(image, { width, height, rgba: pixels });
});

test("an RGB PNG reads back as opaque RGBA", () => {
  const [width, height] = [5, 5];
  const pixels = noise(width * height * 3);
  const image = decodePng(png({ width, height, channels: 3, pixels, filters: [4, 3, 2, 1, 0] }));
  for (let index = 0; index < width * height; index += 1) {
    assert.deepEqual([...image.rgba.subarray(index * 4, index * 4 + 4)], [...pixels.subarray(index * 3, index * 3 + 3), 255]);
  }
});

test("PNGs the reader does not handle are refused", () => {
  const plain = { width: 2, height: 2, channels: 4, pixels: noise(16), filters: [0] };
  assert.throws(() => decodePng(Buffer.from("BM not a png")), /not a PNG file/);
  assert.throws(() => decodePng(png({ ...plain, depth: 16 })), /only 8-bit/);
  assert.throws(() => decodePng(png({ ...plain, interlace: 1 })), /only 8-bit/);
  assert.throws(() => decodePng(png({ ...plain, filters: [5] })), /unknown PNG filter 5/);
  assert.throws(() => decodePng(png(plain).subarray(0, 40)), /cut short/);
});

test("the crop is the top left of the picture, laid on white", () => {
  // 3 by 1: opaque red, half-transparent black, transparent green.
  const image = { width: 3, height: 1, rgba: Buffer.from([255, 0, 0, 255, 0, 0, 0, 128, 0, 255, 0, 0]) };
  assert.deepEqual([...cropOnWhite(image, 3, 1).rgb], [255, 0, 0, 127, 127, 127, 255, 255, 255]);
  assert.deepEqual([...cropOnWhite(image, 1, 1).rgb], [255, 0, 0]);
  assert.throws(() => cropOnWhite(image, 4, 1), /smaller than the bitmap/);
});

test("softened, the pixels LoadImage keeps at 100% are the averages of 2 by 2 blocks", () => {
  // At half size LoadImage keeps the pixels in odd columns of odd rows.
  const [width, height] = [6, 4];
  const rgb = noise(width * height * 3);
  const soft = soften({ width, height, rgb });
  for (let y = 1; y < height; y += 2) {
    for (let x = 1; x < width; x += 2) {
      for (let channel = 0; channel < 3; channel += 1) {
        const at = (row, column) => rgb[(row * width + column) * 3 + channel];
        const block = at(y - 1, x - 1) + at(y - 1, x) + at(y, x - 1) + at(y, x);
        assert.equal(soft.rgb[(y * width + x) * 3 + channel], Math.round(block / 4));
      }
    }
  }
});

test("the first row and column are softened with their own pixels past the edge", () => {
  // 2 by 2 grey: 0, 40 / 80, 120.
  const rgb = Buffer.from([0, 0, 0, 40, 40, 40, 80, 80, 80, 120, 120, 120]);
  const soft = soften({ width: 2, height: 2, rgb });
  assert.deepEqual([...soft.rgb].filter((_, index) => index % 3 === 0), [0, 20, 40, 60]);
});

test("a BMP is written bottom up in blue, green, red, each row padded to 4 bytes", () => {
  // 1 by 2: red above, blue below.
  const bytes = encodeBmp({ width: 1, height: 2, rgb: Buffer.from([255, 0, 0, 0, 0, 255]) });
  assert.equal(bytes.toString("latin1", 0, 2), "BM");
  assert.equal(bytes.readUInt32LE(2), bytes.length);
  assert.equal(bytes.readUInt32LE(10), 54);
  assert.equal(bytes.length, 54 + 2 * 4);
  assert.deepEqual([...bytes.subarray(54)], [255, 0, 0, 0, 0, 0, 255, 0]);
  assert.deepEqual(bmpSize(bytes), { width: 1, height: 2 });
});

test("a BMP row of 3 pixels takes 12 bytes, of 5 pixels 16", () => {
  for (const [width, stride] of [[3, 12], [5, 16]]) {
    const bytes = encodeBmp({ width, height: 1, rgb: Buffer.alloc(width * 3) });
    assert.equal(bytes.length - 54, stride);
    assert.equal(bytes.readUInt32LE(34), stride);
  }
});

test("what is not a 24-bit BMP is refused", () => {
  const bytes = encodeBmp({ width: 1, height: 1, rgb: Buffer.alloc(3) });
  bytes.writeUInt16LE(32, 28);
  assert.throws(() => bmpSize(bytes), /not a 24-bit BMP/);
  assert.throws(() => bmpSize(Buffer.alloc(54)), /not a 24-bit BMP/);
});

test("the viewBox gives the size, and the squared SVG keeps the picture in its top left", () => {
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 150 57"><rect viewBox="0 0 1 1"/></svg>';
  assert.deepEqual(viewBoxSize(svg), { width: 150, height: 57 });
  assert.equal(squared(svg), svg.replace('viewBox="0 0 150 57"', 'viewBox="0 0 150 150"'));
  assert.throws(() => viewBoxSize('<svg viewBox="10 0 150 57"></svg>'), /root viewBox/);
});

test("each committed bitmap is a 24-bit BMP of twice its SVG's size, its place's at 100% in Chinese", () => {
  // The welcome page's picture is 109 by 193 dialog units, the header's 100 by 35; the Chinese
  // wizard's font (PMingLiU 9 pt) makes a dialog unit 6/4 by 12/8 px at 100% display scaling.
  const wanted = { "installer-sidebar.bmp": [164, 290], "installer-header.bmp": [150, 53] };
  assert.equal(SCALE, 2);
  for (const { source, bitmap } of IMAGES) {
    const { width, height } = viewBoxSize(readFileSync(path.join(WINDOWS, source), "utf8"));
    assert.deepEqual([width, height], wanted[bitmap], source);
    assert.deepEqual(bmpSize(readFileSync(path.join(WINDOWS, bitmap))), { width: width * SCALE, height: height * SCALE });
  }
});

test("the installer settings use these bitmaps and the app's icon", () => {
  const { nsis } = JSON.parse(readFileSync(path.join(ROOT, "src-tauri", "tauri.conf.json"), "utf8")).bundle.windows;
  assert.equal(nsis.sidebarImage, "windows/installer-sidebar.bmp");
  assert.equal(nsis.headerImage, "windows/installer-header.bmp");
  assert.equal(nsis.installerIcon, "icons/icon.ico");
  assert.equal(nsis.uninstallerIcon, "icons/icon.ico");
});
