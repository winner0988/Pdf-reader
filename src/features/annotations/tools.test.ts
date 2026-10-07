import { describe, expect, it } from "vitest";

import {
  dragged,
  keptOnPage,
  keptPointOnPage,
  moved,
  rectOfBox,
  STAMP_HEIGHT_PT,
  STAMP_WIDTH_PT,
  stampRectAt,
  strokePoints,
  thin,
} from "@/features/annotations/tools";
import type { Rotation } from "@/features/shell/model";
import { displayedSize, rectToBox } from "@/features/viewer/layout";
import { LIMITS } from "@/ipc/generated/contract";

const LETTER = { widthPt: 612, heightPt: 792 };

describe("a stroke of the pen", () => {
  it("keeps the first point, the points a little apart, and the last", () => {
    const points = [0, 0.2, 0.4, 1.1, 1.3, 2.4, 2.5].map((x) => ({ x, y: 0 }));
    expect(thin(points, 1).map((point) => point.x)).toEqual([0, 1.1, 2.4, 2.5]);
    // A dot stays a point; a click that moved a hair is a tiny line.
    expect(thin([{ x: 5, y: 5 }], 1)).toEqual([{ x: 5, y: 5 }]);
    expect(thin([{ x: 5, y: 5 }, { x: 5.2, y: 5 }], 1)).toHaveLength(2);
    expect(thin([], 1)).toEqual([]);
  });

  it("never has more points than a drawing may", () => {
    const long = Array.from({ length: LIMITS.maxInkPoints * 3 }, (_, index) => ({ x: index * 1.1, y: 0 }));
    const kept = strokePoints(long);
    expect(kept.length).toBeLessThanOrEqual(LIMITS.maxInkPoints);
    expect(kept[0]).toEqual(long[0]);
    expect(kept.at(-1)!.x).toBeCloseTo(long.at(-1)!.x, 1);
  });

  it("says its points in hundredths of a point", () => {
    expect(strokePoints([{ x: 1.23456, y: 2.00001 }, { x: 10.006, y: 20 }])).toEqual([
      { x: 1.23, y: 2 },
      { x: 10.01, y: 20 },
    ]);
  });
});

describe("placing a stamp", () => {
  it("centers it where the pointer is, in its own shape", () => {
    const rect = stampRectAt({ x: 300, y: 400 }, LETTER);
    expect(rect.x1 - rect.x0).toBeCloseTo(STAMP_WIDTH_PT);
    expect(rect.y1 - rect.y0).toBeCloseTo(STAMP_HEIGHT_PT);
    expect((rect.x0 + rect.x1) / 2).toBeCloseTo(300);
    expect((rect.y0 + rect.y1) / 2).toBeCloseTo(400);
  });

  it("keeps it on the page near an edge, and on a page smaller than it", () => {
    expect(stampRectAt({ x: 5, y: 5 }, LETTER)).toMatchObject({ x0: 0, y0: 0 });
    const far = stampRectAt({ x: 700, y: 900 }, LETTER);
    expect(far.x1).toBeCloseTo(612);
    expect(far.y1).toBeCloseTo(792);
    expect(stampRectAt({ x: 40, y: 20 }, { widthPt: 100, heightPt: 20 })).toMatchObject({ x0: 0, y0: 0 });
  });
});

describe("moving and resizing a box", () => {
  const box = { left: 100, top: 100, width: 200, height: 100 };

  it("moves as far as the pointer", () => {
    expect(moved(box, 10, -5)).toEqual({ left: 110, top: 95, width: 200, height: 100 });
  });

  it("drags a side or a corner, and never below the least size", () => {
    expect(dragged(box, "e", 50, 99, 8, false)).toEqual({ left: 100, top: 100, width: 250, height: 100 });
    expect(dragged(box, "nw", 20, 30, 8, false)).toEqual({ left: 120, top: 130, width: 180, height: 70 });
    expect(dragged(box, "s", 0, -500, 8, false)).toEqual({ left: 100, top: 100, width: 200, height: 8 });
    expect(dragged(box, "w", 500, 0, 8, false)).toEqual({ left: 292, top: 100, width: 8, height: 100 });
  });

  it("keeps the shape of a stamp by its corners, with the opposite corner fixed", () => {
    const grown = dragged(box, "se", 100, 5, 8, true);
    expect(grown.left).toBe(100);
    expect(grown.top).toBe(100);
    expect(grown.width / grown.height).toBeCloseTo(2);
    expect(grown.width).toBeCloseTo(300);
    const shrunk = dragged(box, "nw", 50, 0, 8, true);
    expect(shrunk.left + shrunk.width).toBeCloseTo(300);
    expect(shrunk.top + shrunk.height).toBeCloseTo(200);
    expect(shrunk.width / shrunk.height).toBeCloseTo(2);
    const tiny = dragged(box, "se", -1000, -1000, 8, true);
    expect(Math.min(tiny.width, tiny.height)).toBeCloseTo(8);
  });
});

describe("the rectangle of a box on the screen", () => {
  it("is where the box was taken from, however the page is turned", () => {
    const rect = { x0: 100, y0: 200, x1: 250, y1: 240 };
    for (const rotation of [0, 90, 180, 270] as Rotation[]) {
      const shown = displayedSize(LETTER, rotation);
      const size = { top: 0, width: shown.widthPt * 1.5, height: shown.heightPt * 1.5 };
      const box = rectToBox(rect, LETTER, rotation, size);
      // A quarter turn swaps the sides of what the screen shows.
      expect(box.width).toBeCloseTo(rotation % 180 === 0 ? 225 : 60);
      const back = rectOfBox(box, LETTER, rotation, size);
      expect(back.x0).toBeCloseTo(rect.x0);
      expect(back.y0).toBeCloseTo(rect.y0);
      expect(back.x1).toBeCloseTo(rect.x1);
      expect(back.y1).toBeCloseTo(rect.y1);
    }
  });
});

describe("keeping a point on its page", () => {
  it("leaves one on the page alone and puts one outside on the nearest edge", () => {
    expect(keptPointOnPage({ x: 10, y: 20 }, LETTER)).toEqual({ x: 10, y: 20 });
    expect(keptPointOnPage({ x: -5, y: 900 }, LETTER)).toEqual({ x: 0, y: 792 });
    expect(keptPointOnPage({ x: 700, y: -1 }, LETTER)).toEqual({ x: 612, y: 0 });
  });
});

describe("keeping a rectangle on its page", () => {
  it("shifts it back in, and leaves one larger than the page alone", () => {
    expect(keptOnPage({ x0: -10, y0: 780, x1: 90, y1: 820 }, LETTER)).toEqual({ x0: 0, y0: 752, x1: 100, y1: 792 });
    expect(keptOnPage({ x0: 10, y0: 10, x1: 20, y1: 20 }, LETTER)).toEqual({ x0: 10, y0: 10, x1: 20, y1: 20 });
    expect(keptOnPage({ x0: -5, y0: 0, x1: 700, y1: 10 }, LETTER)).toEqual({ x0: -5, y0: 0, x1: 700, y1: 10 });
  });
});
