import { describe, expect, it } from "vitest";

import type { RasterImage } from "@/ipc/raster";
import {
  CELL_PX,
  COMPARE_WIDTH_PX,
  compareScale,
  diffRegions,
  MAX_COMPARE_HEIGHT_PX,
  MAX_REGIONS,
  MERGE_GAP_CELLS,
  pagePairs,
  PIXEL_THRESHOLD,
} from "./visual";

function makeRaster(width: number, height: number, fill = 0): RasterImage {
  const pixels = new Uint8ClampedArray(width * height * 4);
  pixels.fill(fill);
  // Alpha to 255
  for (let i = 3; i < pixels.length; i += 4) {
    pixels[i] = 255;
  }
  return { width, height, pixels };
}

describe("visual compare pure functions", () => {
  it("pagePairs(3, 5) pairs pages and only b has the last two pages", () => {
    const pairs = pagePairs(3, 5);
    expect(pairs).toEqual([
      { a: 0, b: 0 },
      { a: 1, b: 1 },
      { a: 2, b: 2 },
      { a: null, b: 3 },
      { a: null, b: 4 },
    ]);
  });

  it("compareScale handles width-limited and height-limited scenarios", () => {
    // Normal page (e.g. A4: 595 x 842 pt): width limited
    const a4Scale = compareScale({ widthPt: 500, heightPt: 700 });
    expect(a4Scale).toBe(COMPARE_WIDTH_PX / 500); // 2.0, height becomes 1400 < 3000

    // Extremely tall page (e.g. 500 x 2000 pt): height limited
    const tallScale = compareScale({ widthPt: 500, heightPt: 2000 });
    expect(tallScale).toBe(MAX_COMPARE_HEIGHT_PX / 2000); // 1.5, height becomes 3000
  });

  it("reports no diff for identical images", () => {
    const a = makeRaster(200, 200, 255);
    const b = makeRaster(200, 200, 255);
    const result = diffRegions(a, b);
    expect(result.regions).toHaveLength(0);
    expect(result.truncated).toBe(false);
  });

  it("detects a 20x20 change as one region covering it, aligned to grid cells", () => {
    const a = makeRaster(200, 200, 255);
    const b = makeRaster(200, 200, 255);

    // Modify a 20x20 px block from (10, 10) to (29, 29)
    for (let y = 10; y < 30; y++) {
      for (let x = 10; x < 30; x++) {
        const idx = (y * 200 + x) * 4;
        b.pixels[idx] = 0; // Black instead of white, diff = 255 > 48
      }
    }

    const result = diffRegions(a, b);
    expect(result.regions).toHaveLength(1);
    const region = result.regions[0];
    // x from 10 to 29 spans cells [10//8, 29//8] = [1, 3] -> x: 8, width: (3 - 1 + 1)*8 = 24
    // y from 10 to 29 spans cells [1, 3] -> y: 8, height: 24
    expect(region.x).toBe(8);
    expect(region.y).toBe(8);
    expect(region.width).toBe(24);
    expect(region.height).toBe(24);
    // Outer frame covers (10, 10) to (30, 30)
    expect(region.x).toBeLessThanOrEqual(10);
    expect(region.y).toBeLessThanOrEqual(10);
    expect(region.x + region.width).toBeGreaterThanOrEqual(30);
    expect(region.y + region.height).toBeGreaterThanOrEqual(30);
  });

  it("finds two separate regions sorted by position when far apart", () => {
    const a = makeRaster(300, 300, 255);
    const b = makeRaster(300, 300, 255);

    // Block 1 at (100, 200)
    for (let y = 200; y < 210; y++) {
      for (let x = 100; x < 110; x++) {
        b.pixels[(y * 300 + x) * 4] = 0;
      }
    }

    // Block 2 at (50, 50)
    for (let y = 50; y < 60; y++) {
      for (let x = 50; x < 60; x++) {
        b.pixels[(y * 300 + x) * 4] = 0;
      }
    }

    const result = diffRegions(a, b);
    expect(result.regions).toHaveLength(2);
    // Sorted by y then x: Block 2 (y=48) should come before Block 1 (y=200)
    expect(result.regions[0].y).toBeLessThan(result.regions[1].y);
    expect(result.regions[0].x).toBe(48); // 50 // 8 * 8
    expect(result.regions[1].y).toBe(200); // 200 // 8 * 8
  });

  it("merges two blocks separated by less than MERGE_GAP_CELLS into one region", () => {
    const a = makeRaster(200, 200, 255);
    const b = makeRaster(200, 200, 255);

    // Block 1: cell c=1 (x=8..15), r=1 (y=8..15)
    b.pixels[(8 * 200 + 8) * 4] = 0;
    // Block 2: cell c=3 (x=24..31), r=1 (y=8..15). Gap between cell 1 and cell 3 is 3 - 1 - 1 = 1 cell <= MERGE_GAP_CELLS (2)
    b.pixels[(8 * 200 + 24) * 4] = 0;

    const result = diffRegions(a, b);
    expect(result.regions).toHaveLength(1);
    expect(result.regions[0].x).toBe(8);
    expect(result.regions[0].width).toBe((3 - 1 + 1) * CELL_PX); // 24 px
  });

  it("ignores color differences below threshold (e.g. diff of 20)", () => {
    const a = makeRaster(100, 100, 100);
    const b = makeRaster(100, 100, 100);

    // Modify with diff 20 (< PIXEL_THRESHOLD of 48)
    for (let y = 10; y < 30; y++) {
      for (let x = 10; x < 30; x++) {
        const idx = (y * 100 + x) * 4;
        b.pixels[idx] = 120; // diff = 20
        b.pixels[idx + 1] = 120;
        b.pixels[idx + 2] = 120;
      }
    }

    const result = diffRegions(a, b);
    expect(result.regions).toHaveLength(0);
  });

  it("handles height difference: > 2px adds extra region at bottom, 1px difference ignored", () => {
    // 1 px height difference: ignored (rounding)
    const a1 = makeRaster(100, 100, 255);
    const b1 = makeRaster(100, 101, 255);
    const res1 = diffRegions(a1, b1);
    expect(res1.regions).toHaveLength(0);

    // 100 px height difference: adds one extra region at bottom
    const a2 = makeRaster(100, 100, 255);
    const b2 = makeRaster(100, 200, 255);
    const res2 = diffRegions(a2, b2);
    expect(res2.regions).toHaveLength(1);
    expect(res2.regions[0]).toEqual({
      x: 0,
      y: 100,
      width: 100,
      height: 100,
    });
  });

  it("truncates and marks truncated when difference regions exceed MAX_REGIONS", () => {
    // Grid with many separated cells
    const a = makeRaster(800, 800, 255);
    const b = makeRaster(800, 800, 255);

    // Place isolated dirty cells separated by 4 cells (gap = 3 > MERGE_GAP_CELLS 2)
    let count = 0;
    for (let r = 0; r < 50; r += 4) {
      for (let c = 0; c < 50; c += 4) {
        b.pixels[(r * CELL_PX * 800 + c * CELL_PX) * 4] = 0;
        count++;
      }
    }
    expect(count).toBeGreaterThan(MAX_REGIONS);

    const result = diffRegions(a, b);
    expect(result.truncated).toBe(true);
    expect(result.regions).toHaveLength(MAX_REGIONS);
  });
});
