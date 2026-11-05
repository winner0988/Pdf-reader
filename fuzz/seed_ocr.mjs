#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";

const dir = process.argv[2] || "fuzz/corpus/ocr";
fs.mkdirSync(dir, { recursive: true });

function makeSeed(filename, width, height, ppi, draw) {
  const total = width * height;
  const buf = Buffer.alloc(7 + total, 255);
  buf[0] = 0; // kind 0: picture
  buf.writeUInt16LE(width, 1);
  buf.writeUInt16LE(height, 3);
  buf.writeUInt16LE(ppi, 5);
  if (draw) draw(buf.subarray(7), width, height);
  fs.writeFileSync(path.join(dir, filename), buf);
}

// 1. Blank picture: 64x32 at 200 ppi
makeSeed("picture-blank-64x32.bin", 64, 32, 200);

// 2. Simple marks/bars: 64x32 at 200 ppi
makeSeed("picture-bars-64x32.bin", 64, 32, 200, (pixels, w) => {
  for (let y = 10; y < 22; y++) {
    for (let x = 10; x < 15; x++) pixels[y * w + x] = 0;
    for (let x = 25; x < 30; x++) pixels[y * w + x] = 0;
    for (let x = 40; x < 45; x++) pixels[y * w + x] = 0;
  }
});

// 3. Blank picture: 100x40 at 72 ppi
makeSeed("picture-blank-100x40.bin", 100, 40, 72);
