#!/usr/bin/env node
// Rebuilds the icon files in src-tauri/icons from their SVG sources: the app icon of both palettes
// and the PDF file icon (docs/ux/visual-style.md, sections 8 and 8.1). The Tauri CLI draws every
// size from one picture. At 32 px and below the guard line of the full app icon turns into a
// smudge, so the app icon's small sizes come from a simplified second source. The PDF file icon has
// a source for each of its small sizes, drawn on that size's pixel grid, so that the letters of its
// "PDF" label are whole pixels.
//
//   node scripts/release/app-icons.mjs
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const ICONS = path.join(ROOT, "src-tauri", "icons");
/** The sizes of icon.ico drawn from the simplified source. */
export const SMALL_SIZES = [16, 24, 32];
/** The sizes icon.ico must have: what Windows asks for at 100% to 400% scaling. */
export const ICO_SIZES = [16, 24, 32, 48, 64, 256];
/**
 * The sizes of pdf-file.ico drawn from a source of their own, `pdf-file-<size>.svg`. 20 px is the
 * small icon at 125% scaling: drawn from another size, its letters would blur.
 */
export const PDF_FILE_PIXEL_SIZES = [16, 20, 24, 32];
/** The sizes pdf-file.ico must have. */
export const PDF_FILE_SIZES = [16, 20, 24, 32, 48, 64, 256];

const DIRECTORY = 6;
const ENTRY = 16;

/** The pictures of an .ico file: `{ size, head, data }`, `head` being the entry's first 8 bytes. */
export function readIco(bytes) {
  if (bytes.length < DIRECTORY || bytes.readUInt16LE(0) !== 0 || bytes.readUInt16LE(2) !== 1) {
    throw new Error("not an .ico file");
  }
  const count = bytes.readUInt16LE(4);
  if (bytes.length < DIRECTORY + count * ENTRY) throw new Error("the .ico directory is cut short");
  const pictures = [];
  for (let index = 0; index < count; index += 1) {
    const at = DIRECTORY + index * ENTRY;
    const length = bytes.readUInt32LE(at + 8);
    const offset = bytes.readUInt32LE(at + 12);
    if (offset + length > bytes.length) throw new Error("an .ico picture is cut short");
    pictures.push({
      // A width of 0 stands for 256.
      size: bytes[at] === 0 ? 256 : bytes[at],
      head: Buffer.from(bytes.subarray(at, at + 8)),
      data: Buffer.from(bytes.subarray(offset, offset + length)),
    });
  }
  return pictures;
}

/** An .ico file with `pictures`, in that order. */
export function writeIco(pictures) {
  const directory = Buffer.alloc(DIRECTORY + pictures.length * ENTRY);
  directory.writeUInt16LE(1, 2);
  directory.writeUInt16LE(pictures.length, 4);
  let offset = directory.length;
  pictures.forEach((picture, index) => {
    const at = DIRECTORY + index * ENTRY;
    picture.head.copy(directory, at);
    directory.writeUInt32LE(picture.data.length, at + 8);
    directory.writeUInt32LE(offset, at + 12);
    offset += picture.data.length;
  });
  return Buffer.concat([directory, ...pictures.map((picture) => picture.data)]);
}

/** `full`'s pictures, those of `smallSizes` replaced by `small`'s; both are .ico files. */
export function mergeIco(full, small, smallSizes = SMALL_SIZES) {
  const simplified = new Map(readIco(small).map((picture) => [picture.size, picture]));
  return writeIco(
    readIco(full).map((picture) => {
      if (!smallSizes.includes(picture.size)) return picture;
      const replacement = simplified.get(picture.size);
      if (!replacement) throw new Error(`the simplified icon has no ${picture.size} px picture`);
      return replacement;
    }),
  );
}

const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

/** An .ico picture holding `png`, a PNG file of `size` by `size` px, with the entry the Tauri CLI writes. */
export function pngPicture(size, png) {
  const isPng = png.length >= 24 && png.subarray(0, 8).equals(PNG_SIGNATURE) && png.toString("latin1", 12, 16) === "IHDR";
  if (!isPng) throw new Error("not a PNG file");
  if (png.readUInt32BE(16) !== size || png.readUInt32BE(20) !== size) {
    throw new Error(`the PNG is not ${size} by ${size} px`);
  }
  const head = Buffer.alloc(8);
  // A width of 0 stands for 256; 32 bits per pixel.
  head[0] = size === 256 ? 0 : size;
  head[1] = head[0];
  head.writeUInt16LE(32, 6);
  return { size, head, data: Buffer.from(png) };
}

/** `full`'s pictures, `pictures` taking the place of those of their size or added where it has none; smallest first. */
export function replacePictures(full, pictures) {
  const bySize = new Map(readIco(full).map((picture) => [picture.size, picture]));
  for (const picture of pictures) bySize.set(picture.size, picture);
  return writeIco([...bySize.values()].sort((a, b) => a.size - b.size));
}

/** Draws `source` into `out`: every picture the Tauri CLI makes, or with `size` only a `<size>x<size>.png`. */
function draw(cli, source, out, size) {
  const only = size ? ["--png", String(size)] : [];
  execFileSync(process.execPath, [cli, "icon", path.join(ICONS, source), "--output", out, ...only], { stdio: "inherit" });
}

/** Draws `source` and its simplified `smallSource` into `work`; the folder of the full pictures and the .ico. */
function build(cli, work, name, source, smallSource) {
  const full = path.join(work, name, "full");
  const small = path.join(work, name, "small");
  draw(cli, source, full);
  draw(cli, smallSource, small);
  const ico = mergeIco(readFileSync(path.join(full, "icon.ico")), readFileSync(path.join(small, "icon.ico")));
  return { full, small, ico };
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? "")) {
  const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
  const work = mkdtempSync(path.join(tmpdir(), "app-icons-"));
  try {
    // The blue tile: the icon of the exe, the installer and the shortcuts.
    const blue = build(cli, work, "blue", "app-icon.svg", "app-icon-small.svg");
    for (const name of ["128x128.png", "128x128@2x.png", "icon.png", "icon.icns"]) {
      copyFileSync(path.join(blue.full, name), path.join(ICONS, name));
    }
    copyFileSync(path.join(blue.small, "32x32.png"), path.join(ICONS, "32x32.png"));
    writeFileSync(path.join(ICONS, "icon.ico"), blue.ico);
    // The dark tile: what the window and the notification area show while Windows is in its
    // light mode (src-tauri/src/app_icon.rs). Only Windows uses it, so only the .ico.
    const dark = build(cli, work, "dark", "app-icon-dark.svg", "app-icon-dark-small.svg");
    writeFileSync(path.join(ICONS, "icon-dark.ico"), dark.ico);
    // The PDF file icon: what Explorer shows for a PDF this app opens, the DefaultIcon the
    // installer writes (src-tauri/windows/installer-hooks.nsh). Only Windows uses it, so only the
    // .ico: 48 px and up from pdf-file.svg, each smaller size from its own source.
    const pdfFull = path.join(work, "pdf", "full");
    draw(cli, "pdf-file.svg", pdfFull);
    const pixels = PDF_FILE_PIXEL_SIZES.map((size) => {
      const out = path.join(work, "pdf", String(size));
      draw(cli, `pdf-file-${size}.svg`, out, size);
      return pngPicture(size, readFileSync(path.join(out, `${size}x${size}.png`)));
    });
    const pdf = replacePictures(readFileSync(path.join(pdfFull, "icon.ico")), pixels);
    writeFileSync(path.join(ICONS, "pdf-file.ico"), pdf);
    const sizes = (ico) => readIco(ico).map((picture) => picture.size).join(", ");
    console.log(`src-tauri/icons rebuilt; icon.ico and icon-dark.ico have ${sizes(blue.ico)} px, pdf-file.ico ${sizes(pdf)} px`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}
