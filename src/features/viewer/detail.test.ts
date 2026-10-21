import { describe, expect, it } from "vitest";

import { covers, detailBaseScale, detailRegion, rasterSize, visiblePart } from "@/features/viewer/detail";
import { LIMITS } from "@/ipc/generated/contract";

const LETTER = { widthPt: 612, heightPt: 792 };
/** A 1200 x 800 window at a device pixel ratio of 2. */
const SCREEN = { width: 2400, height: 1600 };

describe("rasterSize", () => {
  it("rounds up, turned or not", () => {
    expect(rasterSize(LETTER, "none", 1)).toEqual({ width: 612, height: 792 });
    expect(rasterSize(LETTER, "cw90", 1.5)).toEqual({ width: 1188, height: 918 });
    expect(rasterSize(LETTER, "cw180", 0.001)).toEqual({ width: 1, height: 1 });
  });

  it("multiplies 32-bit floats as the main process does", () => {
    // 0.6 as a 32-bit float is a little more than 0.6: 5 x 0.6 is just above 3.
    expect(rasterSize({ widthPt: 5, heightPt: 5 }, "none", 0.6)).toEqual({ width: 4, height: 4 });
  });
});

describe("detailBaseScale", () => {
  it("draws pages up to two screens large whole", () => {
    expect(detailBaseScale(LETTER, "none", 1, 2, SCREEN)).toBeNull();
    expect(detailBaseScale(LETTER, "none", 1.25, 2, SCREEN)).toBeNull();
    expect(detailBaseScale(LETTER, "none", 2, 2, { width: 0, height: 0 })).toBeNull();
  });

  it("draws larger pages over the page at a scale the view had before, no larger than the screen", () => {
    // 200%: 3264 x 4224 pixels; underneath, the page at 100%.
    expect(detailBaseScale(LETTER, "none", 2, 2, SCREEN)).toBe(2.667);
    expect(detailBaseScale(LETTER, "cw90", 4, 2, SCREEN)).toBe(2.667);
    // 300%: at 150% the page is still larger than the screen; at 75% it is not.
    expect(detailBaseScale(LETTER, "none", 3, 2, SCREEN)).toBe(2);
    expect(detailBaseScale(LETTER, "none", 20, 2, SCREEN)).toBe(1.667);
  });

  it("draws pages beyond the raster limits in two layers on any screen", () => {
    const large = { width: 8000, height: 4000 };
    // 400%: 3264 x 4224 pixels, within the limits; 500%: 4081 x 5281, beyond them.
    expect(detailBaseScale(LETTER, "none", 4, 1, large)).toBeNull();
    expect(detailBaseScale(LETTER, "none", 5, 1, large)).not.toBeNull();
  });

  it("draws pages too large for a region whole", () => {
    const huge = { widthPt: 1_000_000, heightPt: 1_000 };
    expect(detailBaseScale(huge, "none", 2, 2, SCREEN)).toBeNull();
  });
});

describe("detailRegion", () => {
  // Letter at 200%: a 1632 x 2112 box, a 3264 x 4224 raster.
  const box = { width: 1632, height: 2112 };
  const whole = { width: 3264, height: 4224 };

  it("is the part on screen with a margin, in raster pixels", () => {
    const view = { left: 100, top: 500, width: 1200, height: 800 };
    expect(detailRegion(view, box, whole, 0)).toEqual({ x: 200, y: 1000, width: 2400, height: 1600 });
    expect(detailRegion(view, box, whole)).toEqual({ x: 50, y: 900, width: 2700, height: 1800 });
  });

  it("stays within the page", () => {
    expect(detailRegion({ left: 0, top: 0, width: 1200, height: 800 }, box, whole)).toEqual({
      x: 0,
      y: 0,
      width: 2550,
      height: 1700,
    });
    expect(detailRegion({ left: 432, top: 1312, width: 1200, height: 800 }, box, whole)).toEqual({
      x: 714,
      y: 2524,
      width: 2550,
      height: 1700,
    });
  });

  it("stays within the raster limits, around the middle of the part on screen", () => {
    const giant = { width: 100_000, height: 100_000 };
    const view = { left: 10_000, top: 10_000, width: 5_000, height: 3_000 };
    const region = detailRegion(view, giant, { width: 200_000, height: 200_000 }, 0)!;
    expect(region.width).toBeLessThanOrEqual(LIMITS.maxRasterSidePx);
    expect(region.height).toBeLessThanOrEqual(LIMITS.maxRasterSidePx);
    expect(region.width * region.height).toBeLessThanOrEqual(LIMITS.maxRasterPixels);
    expect(region.x + region.width / 2).toBeCloseTo(25_000, -1);
    expect(region.y + region.height / 2).toBeCloseTo(23_000, -1);
  });

  it("is nothing for nothing on screen", () => {
    expect(detailRegion({ left: 0, top: 0, width: 0, height: 800 }, box, whole)).toBeNull();
  });
});

describe("covers", () => {
  it("needs all of the inner rectangle", () => {
    const outer = { x: 10, y: 10, width: 100, height: 100 };
    expect(covers(outer, outer)).toBe(true);
    expect(covers(outer, { x: 20, y: 20, width: 90, height: 90 })).toBe(true);
    expect(covers(outer, { x: 20, y: 20, width: 91, height: 90 })).toBe(false);
    expect(covers(outer, { x: 9, y: 20, width: 10, height: 10 })).toBe(false);
  });
});

describe("visiblePart", () => {
  it("is the page box within the viewport, in the box's pixels", () => {
    const box = { top: 100, width: 500, height: 700 };
    expect(visiblePart(box, 50, { top: 0, left: 0, width: 300, height: 400 })).toEqual({
      left: 0,
      top: 0,
      width: 250,
      height: 300,
    });
    expect(visiblePart(box, 50, { top: 300, left: 100, width: 300, height: 400 })).toEqual({
      left: 50,
      top: 200,
      width: 300,
      height: 400,
    });
    expect(visiblePart(box, 50, { top: 800, left: 0, width: 300, height: 400 })).toBeNull();
  });
});
