import { describe, expect, it } from "vitest";

import { cropsNothing, firstTooSmall, marginsFromDrag, pagesFor } from "./crop";

describe("pagesFor", () => {
  it("handles selected with deduplication and sorting", () => {
    expect(pagesFor("selected", 5, [3, 1, 3, 0, 8, -1], 2)).toEqual([0, 1, 3]);
  });

  it("handles current page", () => {
    expect(pagesFor("current", 5, [], 2)).toEqual([2]);
    expect(pagesFor("current", 5, [], 5)).toEqual([]);
    expect(pagesFor("current", 5, [], -1)).toEqual([]);
  });

  it("handles all pages", () => {
    expect(pagesFor("all", 4, [], 0)).toEqual([0, 1, 2, 3]);
    expect(pagesFor("all", 0, [], 0)).toEqual([]);
  });

  it("handles odd pages (1-based: 1, 3, 5 -> 0, 2, 4)", () => {
    expect(pagesFor("odd", 5, [], 0)).toEqual([0, 2, 4]);
  });

  it("handles even pages (1-based: 2, 4 -> 1, 3)", () => {
    expect(pagesFor("even", 5, [], 0)).toEqual([1, 3]);
  });
});

describe("marginsFromDrag", () => {
  const size = { widthPt: 100, heightPt: 200 };

  it("calculates margins with scale = 1", () => {
    // Page is 100x200 px at scale 1. Drag from (10, 20) to (90, 170).
    // top = 20, right = 100 - 90 = 10, bottom = 200 - 170 = 30, left = 10.
    const margins = marginsFromDrag({ x: 10, y: 20 }, { x: 90, y: 170 }, size, 1);
    expect(margins).toEqual({
      top: 20,
      right: 10,
      bottom: 30,
      left: 10,
    });
  });

  it("handles inverted coordinates (dragged bottom-right to top-left)", () => {
    const margins = marginsFromDrag({ x: 90, y: 170 }, { x: 10, y: 20 }, size, 1);
    expect(margins).toEqual({
      top: 20,
      right: 10,
      bottom: 30,
      left: 10,
    });
  });

  it("clamps to page boundaries when dragged outside", () => {
    const margins = marginsFromDrag({ x: -20, y: -10 }, { x: 150, y: 250 }, size, 1);
    expect(margins).toEqual({
      top: 0,
      right: 0,
      bottom: 0,
      left: 0,
    });
  });

  it("scales correctly with scale != 1", () => {
    // scale = 2: page is 200x400 px. Drag from (20, 40) to (180, 340).
    // left = 20/2 = 10, top = 40/2 = 20, right = (200 - 180)/2 = 10, bottom = (400 - 340)/2 = 30.
    const margins = marginsFromDrag({ x: 20, y: 40 }, { x: 180, y: 340 }, size, 2);
    expect(margins).toEqual({
      top: 20,
      right: 10,
      bottom: 30,
      left: 10,
    });
  });
});

describe("firstTooSmall", () => {
  const sizes = [
    { widthPt: 100, heightPt: 100 },
    { widthPt: 200, heightPt: 200 },
  ];

  it("returns null when all remaining sides >= 36 pt", () => {
    // 100 - 30 - 30 = 40 >= 36
    const margins = { top: 30, right: 30, bottom: 30, left: 30 };
    expect(firstTooSmall([0, 1], sizes, margins)).toBeNull();
  });

  it("returns the first page index whose width is < 36 pt", () => {
    // 100 - 40 - 30 = 30 < 36
    const margins = { top: 10, right: 40, bottom: 10, left: 30 };
    expect(firstTooSmall([1, 0], sizes, margins)).toBe(0);
  });

  it("returns the first page index whose height is < 36 pt", () => {
    const margins = { top: 35, right: 10, bottom: 35, left: 10 };
    expect(firstTooSmall([0, 1], sizes, margins)).toBe(0);
  });
});

describe("cropsNothing", () => {
  it("returns true only when all 4 margins are zero", () => {
    expect(cropsNothing({ top: 0, right: 0, bottom: 0, left: 0 })).toBe(true);
    expect(cropsNothing({ top: 1, right: 0, bottom: 0, left: 0 })).toBe(false);
    expect(cropsNothing({ top: 0, right: 1, bottom: 0, left: 0 })).toBe(false);
    expect(cropsNothing({ top: 0, right: 0, bottom: 1, left: 0 })).toBe(false);
    expect(cropsNothing({ top: 0, right: 0, bottom: 0, left: 1 })).toBe(false);
  });
});
