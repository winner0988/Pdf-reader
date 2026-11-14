#!/usr/bin/env node
// Rebuilds the installer's bitmaps in src-tauri/windows from their SVG sources
// (docs/ux/visual-style.md, section 9). NSIS takes only BMP files: the Tauri CLI draws each SVG
// as a PNG, which this script crops to the picture's own size, lays on white and writes as a
// 24-bit BMP. No other tool or dependency is needed.
//
//   node scripts/release/installer-images.mjs
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { inflateSync } from "node:zlib";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const WINDOWS = path.join(ROOT, "src-tauri", "windows");

/**
 * The bitmaps, drawn at twice the size of their SVG's viewBox. The SVGs are the size of their
 * places on the Traditional Chinese wizard's pages at 100% display scaling; the places' sizes
 * follow the wizard's font, so they are not quite in proportion at other scalings or in English
 * (docs/ux/visual-style.md, section 9). NSIS stretches a bitmap to its place with user32's
 * LoadImage, which takes the nearest pixel and does not smooth: where the place is exactly half
 * the bitmap, it keeps the pixels in odd columns of odd rows (counted from 0). Each pixel is made
 * the average of the 2 by 2 pixels from it to its upper left ([`soften`]), so the pixels kept
 * there are those of the picture drawn at its own size, edges smoothed, and at other sizes the
 * picture's edges stay smooth.
 */
export const IMAGES = [
  { source: "installer-sidebar.svg", bitmap: "installer-sidebar.bmp" },
  { source: "installer-header.svg", bitmap: "installer-header.bmp" },
];
export const SCALE = 2;

const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

/** The size an SVG's root `viewBox="0 0 W H"` gives. */
export function viewBoxSize(svg) {
  const root = svg.match(/<svg\b[^>]*>/);
  const box = root?.[0].match(/\bviewBox="0 0 (\d+) (\d+)"/);
  if (!box) throw new Error('the SVG must have a root viewBox="0 0 <width> <height>"');
  return { width: Number(box[1]), height: Number(box[2]) };
}

/**
 * The SVG with a square viewBox of its larger side, the picture in its top left corner: the
 * Tauri CLI only draws square pictures.
 */
export function squared(svg) {
  const { width, height } = viewBoxSize(svg);
  const side = Math.max(width, height);
  return svg.replace(/(<svg\b[^>]*\bviewBox=")0 0 \d+ \d+"/, `$10 0 ${side} ${side}"`);
}

function paeth(left, up, upLeft) {
  const estimate = left + up - upLeft;
  const toLeft = Math.abs(estimate - left);
  const toUp = Math.abs(estimate - up);
  const toUpLeft = Math.abs(estimate - upLeft);
  if (toLeft <= toUp && toLeft <= toUpLeft) return left;
  return toUp <= toUpLeft ? up : upLeft;
}

/** The pixels of an 8-bit RGBA or RGB PNG that is not interlaced: `{ width, height, rgba }`. */
export function decodePng(bytes) {
  if (!bytes.subarray(0, 8).equals(PNG_SIGNATURE)) throw new Error("not a PNG file");
  let at = 8;
  let header = null;
  let ended = false;
  const data = [];
  while (!ended && at + 8 <= bytes.length) {
    const length = bytes.readUInt32BE(at);
    const type = bytes.toString("latin1", at + 4, at + 8);
    if (at + 12 + length > bytes.length) throw new Error("a PNG chunk is cut short");
    const body = bytes.subarray(at + 8, at + 8 + length);
    if (type === "IHDR") {
      header = {
        width: body.readUInt32BE(0),
        height: body.readUInt32BE(4),
        depth: body[8],
        color: body[9],
        interlace: body[12],
      };
    } else if (type === "IDAT") {
      data.push(body);
    } else if (type === "IEND") {
      ended = true;
    }
    at += 12 + length;
  }
  if (!ended) throw new Error("the PNG is cut short: it has no IEND chunk");
  if (!header) throw new Error("the PNG has no header");
  const channels = { 2: 3, 6: 4 }[header.color];
  if (header.depth !== 8 || !channels || header.interlace !== 0) {
    throw new Error("only 8-bit RGB or RGBA PNGs that are not interlaced are read");
  }
  const { width, height } = header;
  const stride = width * channels;
  const raw = inflateSync(Buffer.concat(data));
  if (raw.length < height * (stride + 1)) throw new Error("the PNG's pixels are cut short");
  const pixels = Buffer.alloc(height * stride);
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const row = y * stride;
    for (let x = 0; x < stride; x += 1) {
      const left = x >= channels ? pixels[row + x - channels] : 0;
      const up = y > 0 ? pixels[row - stride + x] : 0;
      const upLeft = y > 0 && x >= channels ? pixels[row - stride + x - channels] : 0;
      const predicted = [0, left, up, (left + up) >> 1, paeth(left, up, upLeft)][filter];
      if (predicted === undefined) throw new Error(`unknown PNG filter ${filter}`);
      pixels[row + x] = (line[x] + predicted) & 0xff;
    }
  }
  if (channels === 4) return { width, height, rgba: pixels };
  const rgba = Buffer.alloc(width * height * 4, 0xff);
  for (let index = 0; index < width * height; index += 1) pixels.copy(rgba, index * 4, index * 3, index * 3 + 3);
  return { width, height, rgba };
}

/** The top left `width` by `height` pixels of `image`, laid on white: `{ width, height, rgb }`. */
export function cropOnWhite(image, width, height) {
  if (width > image.width || height > image.height) throw new Error("the picture is smaller than the bitmap");
  const rgb = Buffer.alloc(width * height * 3);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const from = (y * image.width + x) * 4;
      const alpha = image.rgba[from + 3];
      for (let channel = 0; channel < 3; channel += 1) {
        const value = image.rgba[from + channel];
        rgb[(y * width + x) * 3 + channel] = Math.round((value * alpha + 255 * (255 - alpha)) / 255);
      }
    }
  }
  return { width, height, rgb };
}

/** Each pixel of `image` the average of the 2 by 2 pixels from it to its upper left (the first
 * column and row use their own pixels for the ones past the edge). */
export function soften({ width, height, rgb }) {
  const out = Buffer.alloc(rgb.length);
  for (let y = 0; y < height; y += 1) {
    const above = Math.max(y - 1, 0);
    for (let x = 0; x < width; x += 1) {
      const left = Math.max(x - 1, 0);
      for (let channel = 0; channel < 3; channel += 1) {
        const at = (row, column) => rgb[(row * width + column) * 3 + channel];
        const sum = at(y, x) + at(y, left) + at(above, x) + at(above, left);
        out[(y * width + x) * 3 + channel] = Math.round(sum / 4);
      }
    }
  }
  return { width, height, rgb: out };
}

/** A 24-bit BMP of `image`, as NSIS reads it: rows bottom up, each padded to 4 bytes. */
export function encodeBmp({ width, height, rgb }) {
  const stride = Math.ceil((width * 3) / 4) * 4;
  const offset = 14 + 40;
  const bytes = Buffer.alloc(offset + stride * height);
  bytes.write("BM", 0, "latin1");
  bytes.writeUInt32LE(bytes.length, 2);
  bytes.writeUInt32LE(offset, 10);
  bytes.writeUInt32LE(40, 14);
  bytes.writeInt32LE(width, 18);
  bytes.writeInt32LE(height, 22);
  bytes.writeUInt16LE(1, 26);
  bytes.writeUInt16LE(24, 28);
  bytes.writeUInt32LE(stride * height, 34);
  // 96 DPI, in pixels per metre.
  bytes.writeInt32LE(3780, 38);
  bytes.writeInt32LE(3780, 42);
  for (let y = 0; y < height; y += 1) {
    const row = offset + (height - 1 - y) * stride;
    for (let x = 0; x < width; x += 1) {
      const from = (y * width + x) * 3;
      // BMP keeps blue, green, red.
      bytes[row + x * 3] = rgb[from + 2];
      bytes[row + x * 3 + 1] = rgb[from + 1];
      bytes[row + x * 3 + 2] = rgb[from];
    }
  }
  return bytes;
}

/** The size of a 24-bit BMP, or an error if it is not one. */
export function bmpSize(bytes) {
  if (bytes.toString("latin1", 0, 2) !== "BM" || bytes.readUInt16LE(28) !== 24) {
    throw new Error("not a 24-bit BMP");
  }
  return { width: bytes.readInt32LE(18), height: bytes.readInt32LE(22) };
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? "")) {
  const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
  const work = mkdtempSync(path.join(tmpdir(), "installer-images-"));
  try {
    for (const { source, bitmap } of IMAGES) {
      const svg = readFileSync(path.join(WINDOWS, source), "utf8");
      const { width, height } = viewBoxSize(svg);
      const side = Math.max(width, height) * SCALE;
      const square = path.join(work, source);
      writeFileSync(square, squared(svg));
      const out = path.join(work, path.basename(source, ".svg"));
      execFileSync(process.execPath, [cli, "icon", square, "--output", out, "--png", String(side)], {
        stdio: "inherit",
      });
      const drawn = decodePng(readFileSync(path.join(out, `${side}x${side}.png`)));
      const picture = soften(cropOnWhite(drawn, width * SCALE, height * SCALE));
      writeFileSync(path.join(WINDOWS, bitmap), encodeBmp(picture));
      console.log(`src-tauri/windows/${bitmap}: ${picture.width} x ${picture.height} px`);
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}
