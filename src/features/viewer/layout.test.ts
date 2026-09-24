import { describe, expect, it } from "vitest";

import {
  CSS_PX_PER_PT,
  PAGE_GAP_PX,
  PAGE_PADDING_PX,
  anchorAt,
  boxToPage,
  contentWidth,
  currentPageAt,
  layoutPages,
  pageLeft,
  pageNear,
  pageToBox,
  rectToBox,
  renderScale,
  rotateAnchor,
  scrollForAnchor,
  scrollTopFor,
  visibleRange,
  zoomFactor,
} from "@/features/viewer/layout";
import type { PageBox } from "@/features/viewer/layout";
import { LIMITS } from "@/ipc/generated/contract";

const LETTER = { widthPt: 612, heightPt: 792 };
const LANDSCAPE = { widthPt: 842, heightPt: 595 };
const letters = (count: number) => Array.from({ length: count }, () => LETTER);

describe("layoutPages", () => {
  it("stacks pages with padding and gaps at the zoom factor", () => {
    const layout = layoutPages([LETTER, LANDSCAPE], 0, 1);
    const letterHeight = 792 * CSS_PX_PER_PT; // 1056 px
    expect(layout.boxes[0]).toEqual({ top: PAGE_PADDING_PX, width: 816, height: letterHeight });
    expect(layout.maxWidth).toBeCloseTo(842 * CSS_PX_PER_PT);
    expect(layout.boxes[1]!.top).toBe(PAGE_PADDING_PX + letterHeight + PAGE_GAP_PX);
    expect(layout.totalHeight).toBeCloseTo(2 * PAGE_PADDING_PX + letterHeight + PAGE_GAP_PX + 595 * CSS_PX_PER_PT);
  });

  it("swaps width and height for quarter turns", () => {
    const [box] = layoutPages([LETTER], 90, 0.5).boxes;
    expect(box!.width).toBeCloseTo(792 * CSS_PX_PER_PT * 0.5);
    expect(box!.height).toBeCloseTo(612 * CSS_PX_PER_PT * 0.5);
  });

  it("an empty document has no height", () => {
    expect(layoutPages([], 0, 1)).toEqual({ boxes: [], totalHeight: 0, maxWidth: 0 });
  });
});

describe("zoomFactor", () => {
  const viewport = { top: 0, left: 0, width: 1000, height: 800 };

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

  it("falls back to 100% before the viewport is measured, and fits within 25%–200%", () => {
    expect(zoomFactor("fitWidth", [LETTER], 0, { top: 0, left: 0, width: 0, height: 0 })).toBe(1);
    expect(zoomFactor("fitWidth", [{ widthPt: 100_000, heightPt: 10 }], 0, viewport)).toBe(0.25);
    expect(zoomFactor("fitWidth", [{ widthPt: 100, heightPt: 100 }], 0, viewport)).toBe(2);
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

describe("horizontal layout", () => {
  it("scrolls sideways only when a page is wider than the viewport", () => {
    const layout = layoutPages([LETTER], 0, 1); // 816 px wide
    expect(contentWidth(layout, 1000)).toBe(1000);
    expect(pageLeft(layout.boxes[0]!, 1000)).toBe(92);
    const zoomed = layoutPages([LETTER], 0, 4); // 3264 px wide
    expect(contentWidth(zoomed, 1000)).toBe(3264 + 2 * PAGE_PADDING_PX);
    expect(pageLeft(zoomed.boxes[0]!, contentWidth(zoomed, 1000))).toBe(PAGE_PADDING_PX);
  });
});

describe("zoom anchoring", () => {
  const pages = letters(100);
  const viewport = { width: 1000, height: 800 };
  const center = { x: 500, y: 400 };

  it("keeps the point under the anchor when zooming from 100% to 400% and back", () => {
    const at100 = layoutPages(pages, 0, 1);
    const scroll100 = { top: scrollTopFor(at100, 50) + 300, left: 0 };
    const anchor = anchorAt(at100, contentWidth(at100, 1000), scroll100, center)!;
    expect(anchor.pageIndex).toBe(49);

    const at400 = layoutPages(pages, 0, 4);
    const width400 = contentWidth(at400, 1000);
    const scroll400 = scrollForAnchor(at400, width400, anchor, center, viewport);
    const again = anchorAt(at400, width400, scroll400, center)!;
    expect(again.pageIndex).toBe(49);
    expect(again.fx).toBeCloseTo(anchor.fx);
    expect(again.fy).toBeCloseTo(anchor.fy);
    expect(scroll400.left).toBeGreaterThan(0); // the wide page scrolls to keep the anchor

    const back = scrollForAnchor(at100, contentWidth(at100, 1000), again, center, viewport);
    expect(back.top).toBeCloseTo(scroll100.top);
    expect(currentPageAt(at100, { top: back.top, height: 800 })).toBe(50);
  });

  it("anchors at the cursor, not only at the center", () => {
    const at100 = layoutPages(pages, 0, 1);
    const cursor = { x: 200, y: 100 };
    const anchor = anchorAt(at100, 1000, { top: 5000, left: 0 }, cursor)!;
    const at200 = layoutPages(pages, 0, 2);
    const width = contentWidth(at200, 1000);
    const scroll = scrollForAnchor(at200, width, anchor, cursor, viewport);
    const again = anchorAt(at200, width, scroll, cursor)!;
    expect([again.pageIndex, again.fx, again.fy].map((v) => Math.round(v * 1000))).toEqual(
      [anchor.pageIndex, anchor.fx, anchor.fy].map((v) => Math.round(v * 1000)),
    );
  });

  it("stays within the scrollable range", () => {
    const layout = layoutPages(letters(2), 0, 1);
    const anchor = { pageIndex: 0, fx: 0, fy: 0 };
    expect(scrollForAnchor(layout, 1000, anchor, { x: 900, y: 700 }, viewport)).toEqual({ top: 0, left: 0 });
    const last = { pageIndex: 1, fx: 1, fy: 1 };
    const scroll = scrollForAnchor(layout, 1000, last, { x: 0, y: 0 }, viewport);
    expect(scroll.top).toBeCloseTo(layout.totalHeight - 800);
  });
});

describe("rotateAnchor", () => {
  const anchor = { pageIndex: 3, fx: 0.2, fy: 0.1 };

  it("turns points on the page with the view", () => {
    // Clockwise: the top-left area moves to the top-right.
    expect(rotateAnchor(anchor, 0, 90)).toEqual({ pageIndex: 3, fx: 0.9, fy: 0.2 });
    const half = rotateAnchor(anchor, 0, 180);
    expect([half.fx, half.fy].map((v) => Math.round(v * 10) / 10)).toEqual([0.8, 0.9]);
    const counter = rotateAnchor(anchor, 90, 0); // 270 clockwise
    expect([counter.fx, counter.fy].map((v) => Math.round(v * 10) / 10)).toEqual([0.1, 0.8]);
  });

  it("four quarter turns change nothing", () => {
    let turned = anchor;
    for (const [from, to] of [[0, 90], [90, 180], [180, 270], [270, 0]] as const) turned = rotateAnchor(turned, from, to);
    expect(turned.fx).toBeCloseTo(anchor.fx);
    expect(turned.fy).toBeCloseTo(anchor.fy);
  });
});

describe("pageToBox", () => {
  const page = { widthPt: 600, heightPt: 800 };
  const at = (rotation: 0 | 90 | 180 | 270, x: number, y: number, scale = 1) => {
    const shown = rotation === 90 || rotation === 270 ? { width: 800 * scale, height: 600 * scale } : { width: 600 * scale, height: 800 * scale };
    return pageToBox({ x, y }, page, rotation, shown);
  };

  it("scales page points to the box", () => {
    expect(at(0, 100, 200)).toEqual({ x: 100, y: 200 });
    expect(at(0, 100, 200, 2)).toEqual({ x: 200, y: 400 });
  });

  it("turns with the view", () => {
    // The page's top-left corner goes to the top right after a clockwise quarter turn...
    expect(at(90, 0, 0)).toEqual({ x: 800, y: 0 });
    // ...to the bottom right after half a turn, and to the bottom left after three quarters.
    expect(at(180, 0, 0)).toEqual({ x: 600, y: 800 });
    expect(at(270, 0, 0)).toEqual({ x: 0, y: 600 });
    // A point near the top of the page stays near the new top edge's side it turned to.
    expect(at(90, 100, 50)).toEqual({ x: 750, y: 100 });
    expect(at(270, 100, 50, 0.5)).toEqual({ x: 25, y: 250 });
  });
});

describe("boxToPage", () => {
  const page = { widthPt: 600, heightPt: 800 };

  it("undoes pageToBox at every rotation and scale", () => {
    for (const rotation of [0, 90, 180, 270] as const) {
      const turned = rotation === 90 || rotation === 270;
      const box = turned ? { width: 1600, height: 1200 } : { width: 1200, height: 1600 };
      for (const point of [{ x: 0, y: 0 }, { x: 100, y: 50 }, { x: 600, y: 800 }, { x: 321.5, y: 17.25 }]) {
        const back = boxToPage(pageToBox(point, page, rotation, box), page, rotation, box);
        expect(back.x).toBeCloseTo(point.x);
        expect(back.y).toBeCloseTo(point.y);
      }
    }
  });

  it("finds the page's top left corner wherever the view turned it", () => {
    expect(boxToPage({ x: 800, y: 0 }, page, 90, { width: 800, height: 600 })).toEqual({ x: 0, y: 0 });
    expect(boxToPage({ x: 0, y: 600 }, page, 270, { width: 800, height: 600 })).toEqual({ x: 0, y: 0 });
  });
});

describe("pageNear", () => {
  const layout = layoutPages(letters(3), 0, 1);
  const [first, second] = layout.boxes as [PageBox, PageBox, PageBox];

  it("finds the page at a height, and the nearer page in a gap", () => {
    expect(pageNear(layout, first.top + 10)).toBe(0);
    expect(pageNear(layout, second.top + 10)).toBe(1);
    const gapStart = first.top + first.height;
    expect(pageNear(layout, gapStart + 1)).toBe(0);
    expect(pageNear(layout, second.top - 1)).toBe(1);
  });

  it("clamps above the first page and below the last", () => {
    expect(pageNear(layout, -100)).toBe(0);
    expect(pageNear(layout, layout.totalHeight + 100)).toBe(2);
    expect(pageNear(layoutPages([], 0, 1), 0)).toBeNull();
  });
});

describe("rectToBox", () => {
  const page = { widthPt: 600, heightPt: 800 };
  const rect = { x0: 100, y0: 50, x1: 300, y1: 70 };

  it("places a link's area in the page box", () => {
    expect(rectToBox(rect, page, 0, { width: 1200, height: 1600 })).toEqual({ left: 200, top: 100, width: 400, height: 40 });
  });

  it("turns with the page: a wide area near the top becomes a tall one near the right edge", () => {
    expect(rectToBox(rect, page, 90, { width: 800, height: 600 })).toEqual({ left: 730, top: 100, width: 20, height: 200 });
    expect(rectToBox(rect, page, 180, { width: 600, height: 800 })).toEqual({ left: 300, top: 730, width: 200, height: 20 });
    expect(rectToBox(rect, page, 270, { width: 800, height: 600 })).toEqual({ left: 50, top: 300, width: 20, height: 200 });
  });
});
