#!/usr/bin/env node
// Rebuilds the icon files in src-tauri/icons from their SVG sources (docs/ux/visual-style.md,
// section 8). The Tauri CLI draws every size from one picture; at 32 px and below the guard line
// of the full picture turns into a smudge, so those sizes come from a simplified second source.
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

function draw(cli, source, out) {
  execFileSync(process.execPath, [cli, "icon", path.join(ICONS, source), "--output", out], { stdio: "inherit" });
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? "")) {
  const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
  const work = mkdtempSync(path.join(tmpdir(), "app-icons-"));
  try {
    const full = path.join(work, "full");
    const small = path.join(work, "small");
    draw(cli, "app-icon.svg", full);
    draw(cli, "app-icon-small.svg", small);
    for (const name of ["128x128.png", "128x128@2x.png", "icon.png", "icon.icns"]) {
      copyFileSync(path.join(full, name), path.join(ICONS, name));
    }
    copyFileSync(path.join(small, "32x32.png"), path.join(ICONS, "32x32.png"));
    const ico = mergeIco(readFileSync(path.join(full, "icon.ico")), readFileSync(path.join(small, "icon.ico")));
    writeFileSync(path.join(ICONS, "icon.ico"), ico);
    const sizes = readIco(ico).map((picture) => picture.size);
    console.log(`src-tauri/icons rebuilt; icon.ico has ${sizes.join(", ")} px`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}
