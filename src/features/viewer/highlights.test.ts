import { describe, expect, it } from "vitest";

import { hitRange, pageOverlay, quadPoints, revealScroll, type Highlight } from "@/features/viewer/highlights";
import { contentWidth, layoutPages, pageLeft } from "@/features/viewer/layout";
import type { Quad } from "@/ipc/generated/contract";

const LETTER = { widthPt: 612, heightPt: 792 };
const AT_100 = { top: 0, width: 816, height: 1056 };

/** A one-line hit from (x, y) to (x + w, y + h) in page points. */
const quad = (x: number, y: number, w = 72, h = 12): Quad => ({
  ul: { x, y },
  ur: { x: x + w, y },
  ll: { x, y: y + h },
  lr: { x: x + w, y: y + h },
});
const hit = (pageIndex: number, ...quads: Quad[]): Highlight => ({ pageIndex, quads });

describe("hitRange", () => {
  const hits = [1, 1, 3, 3, 3, 7].map((page) => hit(page, quad(0, 0)));

  it("finds the hits of one page in the document-ordered list", () => {
    expect(hitRange(hits, 3)).toEqual([2, 5]);
    expect(hitRange(hits, 1)).toEqual([0, 2]);
    expect(hitRange(hits, 7)).toEqual([5, 6]);
  });

  it("is empty for pages without hits", () => {
    expect(hitRange(hits, 0)).toEqual([0, 0]);
    expect(hitRange(hits, 2)).toEqual([2, 2]);
    expect(hitRange(hits, 9)).toEqual([6, 6]);
    expect(hitRange([], 0)).toEqual([0, 0]);
  });
});

describe("quadPoints", () => {
  it("scales page points to the page box", () => {
    expect(quadPoints(quad(72, 72), LETTER, 0, AT_100)).toBe("96,96 192,96 192,112 96,112");
    expect(quadPoints(quad(72, 72), LETTER, 0, { top: 0, width: 408, height: 528 })).toBe("48,48 96,48 96,56 48,56");
  });

  it("follows the page when it is rotated", () => {
    const turned = { top: 0, width: 1056, height: 816 };
    // Top left of the page ends up top right, running downwards.
    expect(quadPoints(quad(72, 72), LETTER, 90, turned)).toBe("960,96 960,192 944,192 944,96");
    // Upside down: bottom right, running leftwards.
    expect(quadPoints(quad(72, 72), LETTER, 180, AT_100)).toBe("720,960 624,960 624,944 720,944");
    // Bottom left, running upwards.
    expect(quadPoints(quad(72, 72), LETTER, 270, turned)).toBe("96,720 96,624 112,624 112,720");
  });
});

describe("pageOverlay", () => {
  const hits = [hit(0, quad(0, 0)), hit(2, quad(72, 72)), hit(2, quad(72, 144), quad(72, 156))];

  it("draws every hit of the page and outlines the current one", () => {
    const overlay = pageOverlay({ hits, current: 2 }, 2, LETTER, 0, AT_100);
    expect(overlay?.all).toHaveLength(3);
    expect(overlay?.current).toEqual([
      quadPoints(quad(72, 144), LETTER, 0, AT_100),
      quadPoints(quad(72, 156), LETTER, 0, AT_100),
    ]);
  });

  it("has nothing to outline when the current hit is on another page", () => {
    expect(pageOverlay({ hits, current: 0 }, 2, LETTER, 0, AT_100)?.current).toEqual([]);
    expect(pageOverlay({ hits, current: 0 }, 1, LETTER, 0, AT_100)).toBeNull();
  });
});

describe("revealScroll", () => {
  const pages = [LETTER, LETTER, LETTER];
  const layout = layoutPages(pages, 0, 1);
  const width = contentWidth(layout, 1000);

  it("puts the hit a third of the way down the view", () => {
    const viewport = { top: 0, left: 0, width: 1000, height: 900 };
    const target = revealScroll(hit(1, quad(72, 72)), pages, 0, layout, width, viewport);
    expect(target).toEqual({ top: layout.boxes[1]!.top + 96 - 300 });
  });

  it("scrolls sideways only when the hit is out of view", () => {
    const zoomed = layoutPages(pages, 0, 4);
    const wide = contentWidth(zoomed, 1000);
    const viewport = { top: 0, left: 0, width: 1000, height: 900 };
    const target = revealScroll(hit(0, quad(500, 72)), pages, 0, zoomed, wide, viewport);
    const hitLeft = pageLeft(zoomed.boxes[0]!, wide) + 500 * (4 * 96) / 72;
    expect(target?.left).toBeCloseTo(hitLeft - 1000 / 3);
  });

  it("does nothing for unknown pages or empty hits", () => {
    const viewport = { top: 0, left: 0, width: 1000, height: 900 };
    expect(revealScroll(hit(5, quad(0, 0)), pages, 0, layout, width, viewport)).toBeNull();
    expect(revealScroll(hit(0), pages, 0, layout, width, viewport)).toBeNull();
  });
});
