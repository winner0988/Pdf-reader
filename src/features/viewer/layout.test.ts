import { describe, expect, it } from "vitest";

import {
  CSS_PX_PER_PT,
  PAGE_GAP_PX,
  PAGE_PADDING_PX,
  currentPageAt,
  layoutPages,
  renderScale,
  scrollTopFor,
  visibleRange,
  zoomFactor,
} from "@/features/viewer/layout";
import { LIMITS } from "@/ipc/generated/contract";

const LETTER = { widthPt: 612, heightPt: 792 };
const LANDSCAPE = { widthPt: 842, heightPt: 595 };
const letters = (count: number) => Array.from({ length: count }, () => LETTER);

describe("layoutPages", () => {
  it("stacks pages with padding and gaps at the zoom factor", () => {
    const layout = layoutPages([LETTER, LANDSCAPE], 0, 1);
    const letterHeight = 792 * CSS_PX_PER_PT; // 1056 px
    expect(layout.boxes[0]).toEqual({ top: PAGE_PADDING_PX, width: 816, height: letterHeight });
    expect(layout.boxes[1]!.top).toBe(PAGE_PADDING_PX + letterHeight + PAGE_GAP_PX);
    expect(layout.totalHeight).toBeCloseTo(2 * PAGE_PADDING_PX + letterHeight + PAGE_GAP_PX + 595 * CSS_PX_PER_PT);
  });

  it("swaps width and height for quarter turns", () => {
    const [box] = layoutPages([LETTER], 90, 0.5).boxes;
    expect(box!.width).toBeCloseTo(792 * CSS_PX_PER_PT * 0.5);
    expect(box!.height).toBeCloseTo(612 * CSS_PX_PER_PT * 0.5);
  });

  it("an empty document has no height", () => {
    expect(layoutPages([], 0, 1)).toEqual({ boxes: [], totalHeight: 0 });
  });
});

describe("zoomFactor", () => {
  const viewport = { top: 0, width: 1000, height: 800 };

  it("uses percentages directly", () => {
    expect(zoomFactor(150, [LETTER], 0, viewport)).toBe(1.5);
  });

  it("fits the widest page to the width", () => {
    const factor = zoomFactor("fitWidth", [LETTER, LANDSCAPE], 0, viewport);
    expect(842 * CSS_PX_PER_PT * factor).toBeCloseTo(1000 - 2 * PAGE_PADDING_PX);
  });

  it("fits the whole page for fitPage", () => {
    const factor = zoomFactor("fitPage", [LETTER], 0, viewport);
    expect(792 * CSS_PX_PER_PT * factor).toBeCloseTo(800 - 2 * PAGE_PADDING_PX);
  });

  it("falls back to 100% before the viewport is measured, and stays within 25%–800%", () => {
    expect(zoomFactor("fitWidth", [LETTER], 0, { top: 0, width: 0, height: 0 })).toBe(1);
    expect(zoomFactor("fitWidth", [{ widthPt: 100_000, heightPt: 10 }], 0, viewport)).toBe(0.25);
    expect(zoomFactor("fitWidth", [{ widthPt: 1, heightPt: 1 }], 0, viewport)).toBe(8);
  });
});

describe("visibleRange", () => {
  const layout = layoutPages(letters(100), 0, 1); // each page 1056 px + 12 px gap
  const pitch = 1056 + PAGE_GAP_PX;

  it("covers the visible pages plus two on each side", () => {
    expect(visibleRange(layout, { top: 0, height: 800 })).toEqual({ first: 0, last: 2 });
    // Pages 10 and 11 (0-based) are visible.
    expect(visibleRange(layout, { top: PAGE_PADDING_PX + pitch * 10 + 500, height: 800 })).toEqual({
      first: 8,
      last: 13,
    });
  });

  it("is clamped at the end of the document", () => {
    const range = visibleRange(layout, { top: layout.totalHeight - 800, height: 800 });
    expect(range.last).toBe(99);
    expect(range.first).toBe(97);
  });

  it("handles documents with thousands of pages quickly", () => {
    const huge = layoutPages(letters(100_000), 0, 1);
    const started = performance.now();
    for (let i = 0; i < 1000; i++) visibleRange(huge, { top: i * 50_000, height: 900 });
    expect(performance.now() - started).toBeLessThan(200);
  });

  it("is empty without pages", () => {
    expect(visibleRange(layoutPages([], 0, 1), { top: 0, height: 800 })).toEqual({ first: 0, last: -1 });
  });
});

describe("currentPageAt", () => {
  const layout = layoutPages(letters(10), 0, 1);

  it("is the page 30% down the viewport", () => {
    expect(currentPageAt(layout, { top: 0, height: 800 })).toBe(1);
    expect(currentPageAt(layout, { top: scrollTopFor(layout, 4), height: 800 })).toBe(4);
  });

  it("is the last page when scrolled to the end", () => {
    expect(currentPageAt(layout, { top: layout.totalHeight - 800, height: 800 })).toBe(10);
  });
});

describe("renderScale", () => {
  it("includes the device pixel ratio and rounds to 1/1000", () => {
    expect(renderScale(1, 1)).toBe(1.333);
    expect(renderScale(1.5, 2)).toBe(4);
    expect(renderScale(1, Number.NaN)).toBe(1.333);
  });

  it("stays within the contract limits", () => {
    expect(renderScale(100, 3)).toBe(LIMITS.maxRenderScale);
    expect(renderScale(0.0001, 1)).toBe(LIMITS.minRenderScale);
  });
});
