// Page geometry for the virtual scroller (MVP-07, docs/architecture/rendering.md). Pure functions:
// sizes in CSS pixels, page indexes 0-based unless stated otherwise.

import type { PageSize, Rotation, Zoom } from "@/features/shell/model";
import { LIMITS } from "@/ipc/generated/contract";

/** CSS pixels per PDF point at 100%: a page shows at its printed size on a 96 dpi screen. */
export const CSS_PX_PER_PT = 96 / 72;
/** Space around the pages and between them. */
export const PAGE_PADDING_PX = 16;
export const PAGE_GAP_PX = 12;
/** Pages kept rendered above and below the visible ones. */
export const OVERSCAN_PAGES = 2;
/** Fit modes stay within the zoom menu's range (25%–800%). */
const MIN_FIT_FACTOR = 0.25;
const MAX_FIT_FACTOR = 8;

export type Viewport = { top: number; width: number; height: number };
export type PageBox = { top: number; width: number; height: number };
export type Layout = { boxes: PageBox[]; totalHeight: number };

/** Page size as displayed, in points: quarter turns swap width and height. */
export function displayedSize(page: PageSize, rotation: Rotation): PageSize {
  return rotation === 90 || rotation === 270 ? { widthPt: page.heightPt, heightPt: page.widthPt } : page;
}

/**
 * CSS pixels per CSS pixel at 100% (1 = 100%). Fit modes use the largest page so the factor
 * does not jump while scrolling through mixed page sizes. An unmeasured viewport counts as 100%.
 */
export function zoomFactor(zoom: Zoom, pages: PageSize[], rotation: Rotation, viewport: Viewport): number {
  if (typeof zoom === "number") return zoom / 100;
  if (viewport.width <= 0 || pages.length === 0) return 1;
  let widest = 0;
  let tallest = 0;
  for (const page of pages) {
    const size = displayedSize(page, rotation);
    widest = Math.max(widest, size.widthPt);
    tallest = Math.max(tallest, size.heightPt);
  }
  const available = (px: number) => Math.max(px - 2 * PAGE_PADDING_PX, 1);
  let factor = available(viewport.width) / (widest * CSS_PX_PER_PT);
  if (zoom === "fitPage" && viewport.height > 0) {
    factor = Math.min(factor, available(viewport.height) / (tallest * CSS_PX_PER_PT));
  }
  return Math.min(Math.max(factor, MIN_FIT_FACTOR), MAX_FIT_FACTOR);
}

/** Where every page goes in the scrolled content: stacked vertically, centered horizontally. */
export function layoutPages(pages: PageSize[], rotation: Rotation, factor: number): Layout {
  const boxes: PageBox[] = [];
  let top = PAGE_PADDING_PX;
  for (const page of pages) {
    const size = displayedSize(page, rotation);
    const box = {
      top,
      width: size.widthPt * CSS_PX_PER_PT * factor,
      height: size.heightPt * CSS_PX_PER_PT * factor,
    };
    boxes.push(box);
    top += box.height + PAGE_GAP_PX;
  }
  const totalHeight = boxes.length === 0 ? 0 : top - PAGE_GAP_PX + PAGE_PADDING_PX;
  return { boxes, totalHeight };
}

/** First page whose bottom edge is below `y`. */
function firstBelow(boxes: PageBox[], y: number): number {
  let low = 0;
  let high = boxes.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    const box = boxes[middle]!;
    if (box.top + box.height <= y) low = middle + 1;
    else high = middle;
  }
  return low;
}

/**
 * Pages to keep rendered: those intersecting the viewport plus `overscan` on each side.
 * Returns an empty range (`last < first`) for a document without pages.
 */
export function visibleRange(
  layout: Layout,
  viewport: Pick<Viewport, "top" | "height">,
  overscan = OVERSCAN_PAGES,
): { first: number; last: number } {
  const count = layout.boxes.length;
  if (count === 0) return { first: 0, last: -1 };
  const bottom = viewport.top + viewport.height;
  const first = Math.min(firstBelow(layout.boxes, viewport.top), count - 1);
  let last = first;
  while (last + 1 < count && layout.boxes[last + 1]!.top < bottom) last++;
  return { first: Math.max(first - overscan, 0), last: Math.min(last + overscan, count - 1) };
}

/** The 1-based page being read: the one at 30% of the viewport height, or the last page at the end. */
export function currentPageAt(layout: Layout, viewport: Pick<Viewport, "top" | "height">): number {
  const count = layout.boxes.length;
  if (count === 0) return 1;
  if (viewport.top + viewport.height >= layout.totalHeight - 1 && viewport.top > 0) return count;
  const index = firstBelow(layout.boxes, viewport.top + viewport.height * 0.3);
  return Math.min(index, count - 1) + 1;
}

/** Scroll position that puts a 1-based page at the top of the viewport. */
export function scrollTopFor(layout: Layout, page: number): number {
  const box = layout.boxes[page - 1];
  return box ? Math.max(box.top - PAGE_PADDING_PX / 2, 0) : 0;
}

/**
 * Worker render scale (device pixels per PDF point) for a zoom factor, rounded to 1/1000 so
 * that equal views hit the main process cache, and kept within the contract's limits.
 */
export function renderScale(factor: number, devicePixelRatio: number): number {
  const ratio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  const scale = Math.round(factor * CSS_PX_PER_PT * ratio * 1000) / 1000;
  return Math.min(Math.max(scale, LIMITS.minRenderScale), LIMITS.maxRenderScale);
}
