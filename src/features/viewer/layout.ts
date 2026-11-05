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
/** Fit modes stay between 25% and 200% (docs/ux/screen-map.md, section 7). */
const MIN_FIT_FACTOR = 0.25;
const MAX_FIT_FACTOR = 2;

export type PageLayoutMode = "single" | "twoPage" | "twoPageCover";

export type Viewport = { top: number; left: number; width: number; height: number };
export type PageBox = { top: number; width: number; height: number; dx?: number };
export type LayoutRow = { top: number; height: number; firstPageIndex: number; pageCount: number };
export type Layout = { boxes: PageBox[]; rows: LayoutRow[]; totalHeight: number; maxWidth: number };
/** A point relative to the viewport's top-left corner, in CSS pixels. */
export type ViewportPoint = { x: number; y: number };
/** A place in the document: a page and a position on it, as fractions of its displayed size. */
export type Anchor = { pageIndex: number; fx: number; fy: number };

/** Page size as displayed, in points: quarter turns swap width and height. */
export function displayedSize(page: PageSize, rotation: Rotation): PageSize {
  return rotation === 90 || rotation === 270 ? { widthPt: page.heightPt, heightPt: page.widthPt } : page;
}

/**
 * CSS pixels per CSS pixel at 100% (1 = 100%). Fit modes use the largest page so the factor
 * does not jump while scrolling through mixed page sizes. An unmeasured viewport counts as 100%.
 */
export function zoomFactor(
  zoom: Zoom,
  pages: PageSize[],
  rotation: Rotation,
  viewport: Viewport,
  mode: PageLayoutMode = "single",
): number {
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
  let factor: number;
  if (mode === "single") {
    factor = available(viewport.width) / (widest * CSS_PX_PER_PT);
  } else {
    // In two-page modes, a row consists of two columns (each with width equal to widest page) plus the gap between them.
    const availableWidth = Math.max(available(viewport.width) - PAGE_GAP_PX, 1);
    factor = availableWidth / (2 * widest * CSS_PX_PER_PT);
  }
  if (zoom === "fitPage" && viewport.height > 0) {
    factor = Math.min(factor, available(viewport.height) / (tallest * CSS_PX_PER_PT));
  }
  return Math.min(Math.max(factor, MIN_FIT_FACTOR), MAX_FIT_FACTOR);
}

/** Where every page goes in the scrolled content: stacked vertically, centered horizontally. */
export function layoutPages(
  pages: PageSize[],
  rotation: Rotation,
  factor: number,
  mode: PageLayoutMode = "single",
): Layout {
  if (pages.length === 0) {
    return { boxes: [], rows: [], totalHeight: 0, maxWidth: 0 };
  }

  if (mode === "single") {
    const boxes: PageBox[] = [];
    const rows: LayoutRow[] = [];
    let top = PAGE_PADDING_PX;
    let maxWidth = 0;
    for (let i = 0; i < pages.length; i++) {
      const page = pages[i]!;
      const size = displayedSize(page, rotation);
      const width = size.widthPt * CSS_PX_PER_PT * factor;
      const height = size.heightPt * CSS_PX_PER_PT * factor;
      const box: PageBox = {
        top,
        width,
        height,
        dx: -width / 2,
      };
      boxes.push(box);
      rows.push({
        top,
        height,
        firstPageIndex: i,
        pageCount: 1,
      });
      maxWidth = Math.max(maxWidth, width);
      top += height + PAGE_GAP_PX;
    }
    const totalHeight = top - PAGE_GAP_PX + PAGE_PADDING_PX;
    return { boxes, rows, totalHeight, maxWidth };
  }

  // Two-page modes ("twoPage" | "twoPageCover")
  // Both columns have the width of the widest displayed page.
  let columnWidth = 0;
  for (const page of pages) {
    const size = displayedSize(page, rotation);
    columnWidth = Math.max(columnWidth, size.widthPt * CSS_PX_PER_PT * factor);
  }
  const maxWidth = 2 * columnWidth + PAGE_GAP_PX;

  // Group pages into rows:
  // offset: 1 for twoPageCover, 0 for twoPage
  // Each page i is placed in row r = Math.floor((i + offset) / 2)
  // and column col = (i + offset) % 2 (0: left, 1: right)
  const offset = mode === "twoPageCover" ? 1 : 0;
  const rowGroups: { firstPageIndex: number; pageIndexes: number[] }[] = [];
  for (let i = 0; i < pages.length; i++) {
    const r = Math.floor((i + offset) / 2);
    if (!rowGroups[r]) {
      rowGroups[r] = { firstPageIndex: i, pageIndexes: [] };
    }
    rowGroups[r]!.pageIndexes.push(i);
  }

  const boxes: PageBox[] = new Array(pages.length);
  const rows: LayoutRow[] = [];
  let top = PAGE_PADDING_PX;

  for (const group of rowGroups) {
    if (!group || group.pageIndexes.length === 0) continue;
    let rowHeight = 0;
    const pageItems = group.pageIndexes.map((idx) => {
      const size = displayedSize(pages[idx]!, rotation);
      const width = size.widthPt * CSS_PX_PER_PT * factor;
      const height = size.heightPt * CSS_PX_PER_PT * factor;
      rowHeight = Math.max(rowHeight, height);
      return { idx, width, height };
    });

    const rowTop = top;
    for (const item of pageItems) {
      const col = (item.idx + offset) % 2;
      const pageTop = rowTop + (rowHeight - item.height) / 2;
      const dx =
        col === 0
          ? -PAGE_GAP_PX / 2 - columnWidth + (columnWidth - item.width) / 2
          : PAGE_GAP_PX / 2 + (columnWidth - item.width) / 2;

      boxes[item.idx] = {
        top: pageTop,
        width: item.width,
        height: item.height,
        dx,
      };
    }

    rows.push({
      top: rowTop,
      height: rowHeight,
      firstPageIndex: group.firstPageIndex,
      pageCount: group.pageIndexes.length,
    });

    top += rowHeight + PAGE_GAP_PX;
  }

  const totalHeight = top - PAGE_GAP_PX + PAGE_PADDING_PX;
  return { boxes, rows, totalHeight, maxWidth };
}

/** Width of the scrolled content: the viewport, or wider when a page does not fit (then it scrolls). */
export function contentWidth(layout: Layout, viewportWidth: number): number {
  return Math.max(viewportWidth, layout.maxWidth + 2 * PAGE_PADDING_PX);
}

/**
 * Whether the pages fit the viewport's width, give or take a pixel: then the content just fills
 * the viewport. `clientWidth` is rounded to whole pixels (at 150% display scaling a canvas 904.67
 * pixels wide reads 905), so content given the measured width can be a fraction of a pixel too
 * wide, which shows a horizontal scroll bar at fit width (#83).
 */
export function fitsWidth(layout: Layout, viewportWidth: number): boolean {
  return layout.maxWidth + 2 * PAGE_PADDING_PX <= viewportWidth + 1;
}

/** Left edge of a page: centered in the content or column. */
export function pageLeft(box: PageBox, width: number): number {
  return box.dx !== undefined ? width / 2 + box.dx : (width - box.width) / 2;
}

/** First row whose bottom edge is below `y`. */
function firstRowBelow(rows: LayoutRow[], y: number): number {
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    const row = rows[middle]!;
    if (row.top + row.height <= y) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** The 0-based index of the row containing page `pageIndex`. */
export function rowForPage(layout: Layout, pageIndex: number): number {
  const rows = layout.rows;
  if (rows.length === 0 || pageIndex < 0) return 0;
  if (pageIndex >= layout.boxes.length) return rows.length - 1;
  let low = 0;
  let high = rows.length - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    const row = rows[mid]!;
    if (pageIndex < row.firstPageIndex) {
      high = mid - 1;
    } else if (pageIndex >= row.firstPageIndex + row.pageCount) {
      low = mid + 1;
    } else {
      return mid;
    }
  }
  return Math.min(Math.max(low, 0), rows.length - 1);
}

/** The 0-based index of the row at 30% of the viewport height, or the last row at the end. */
export function currentRowAt(layout: Layout, viewport: Pick<Viewport, "top" | "height">): number {
  const rowCount = layout.rows.length;
  if (rowCount === 0) return 0;
  if (viewport.top + viewport.height >= layout.totalHeight - 1 && viewport.top > 0) return rowCount - 1;
  const index = firstRowBelow(layout.rows, viewport.top + viewport.height * 0.3);
  return Math.min(index, rowCount - 1);
}

/**
 * Pages to keep rendered: rows intersecting the viewport plus `overscan` rows on each side.
 * Returns an empty range (`last < first`) for a document without pages.
 */
export function visibleRange(
  layout: Layout,
  viewport: Pick<Viewport, "top" | "height">,
  overscan = OVERSCAN_PAGES,
): { first: number; last: number } {
  const rowCount = layout.rows.length;
  if (rowCount === 0 || layout.boxes.length === 0) return { first: 0, last: -1 };
  const bottom = viewport.top + viewport.height;
  const firstRow = Math.min(firstRowBelow(layout.rows, viewport.top), rowCount - 1);
  let lastRow = firstRow;
  while (lastRow + 1 < rowCount && layout.rows[lastRow + 1]!.top < bottom) lastRow++;
  const startRow = Math.max(firstRow - overscan, 0);
  const endRow = Math.min(lastRow + overscan, rowCount - 1);
  const first = layout.rows[startRow]!.firstPageIndex;
  const lastRowObj = layout.rows[endRow]!;
  const last = lastRowObj.firstPageIndex + lastRowObj.pageCount - 1;
  return { first, last };
}

/** The document place under `point` (for keeping it in place while zooming or rotating). */
export function anchorAt(
  layout: Layout,
  width: number,
  viewport: Pick<Viewport, "top" | "left">,
  point: ViewportPoint,
): Anchor | null {
  const count = layout.boxes.length;
  if (count === 0) return null;
  const y = viewport.top + point.y;
  const x = viewport.left + point.x;
  const pageIndex = pageNear(layout, y, x, width);
  if (pageIndex === null) return null;
  const box = layout.boxes[pageIndex]!;
  return { pageIndex, fx: (x - pageLeft(box, width)) / box.width, fy: (y - box.top) / box.height };
}

/** Scroll position that puts `anchor` back under `point`, within the scrollable range. */
export function scrollForAnchor(
  layout: Layout,
  width: number,
  anchor: Anchor,
  point: ViewportPoint,
  viewport: Pick<Viewport, "width" | "height">,
): { top: number; left: number } {
  const box = layout.boxes[Math.min(anchor.pageIndex, layout.boxes.length - 1)];
  if (!box) return { top: 0, left: 0 };
  const clamp = (value: number, max: number) => Math.min(Math.max(value, 0), Math.max(max, 0));
  return {
    top: clamp(box.top + anchor.fy * box.height - point.y, layout.totalHeight - viewport.height),
    left: clamp(pageLeft(box, width) + anchor.fx * box.width - point.x, width - viewport.width),
  };
}

/**
 * Where a point on a page ends up when the view turns from `from` to `to` degrees clockwise.
 * Turning a W x H page clockwise by 90 degrees moves (x, y) to (H - y, x); in fractions of the
 * displayed size, (fx, fy) becomes (1 - fy, fx).
 */
export function rotateAnchor(anchor: Anchor, from: Rotation, to: Rotation): Anchor {
  let { fx, fy } = anchor;
  const quarterTurns = (((to - from) / 90) % 4 + 4) % 4;
  for (let turn = 0; turn < quarterTurns; turn++) [fx, fy] = [1 - fy, fx];
  return { pageIndex: anchor.pageIndex, fx, fy };
}

/**
 * Maps a point in page space (PDF points of the unrotated page, origin top left, y down: how
 * the worker reports text positions) to CSS pixels inside the page's box as displayed. A
 * clockwise quarter turn moves (x, y) to (H - y, x).
 */
export function pageToBox(
  point: { x: number; y: number },
  page: PageSize,
  rotation: Rotation,
  box: Pick<PageBox, "width" | "height">,
): { x: number; y: number } {
  const { widthPt: w, heightPt: h } = page;
  let x = point.x;
  let y = point.y;
  if (rotation === 90) [x, y] = [h - point.y, point.x];
  else if (rotation === 180) [x, y] = [w - point.x, h - point.y];
  else if (rotation === 270) [x, y] = [point.y, w - point.x];
  const shown = displayedSize(page, rotation);
  return { x: (x / shown.widthPt) * box.width, y: (y / shown.heightPt) * box.height };
}

/** The inverse of {@link pageToBox}: a point in a page's box, in CSS pixels, as a point in page space. */
export function boxToPage(
  point: { x: number; y: number },
  page: PageSize,
  rotation: Rotation,
  box: Pick<PageBox, "width" | "height">,
): { x: number; y: number } {
  const shown = displayedSize(page, rotation);
  const x = (point.x / box.width) * shown.widthPt;
  const y = (point.y / box.height) * shown.heightPt;
  const { widthPt: w, heightPt: h } = page;
  if (rotation === 90) return { x: y, y: h - x };
  if (rotation === 180) return { x: w - x, y: h - y };
  if (rotation === 270) return { x: w - y, y: x };
  return { x, y };
}

/**
 * The page at content position `(x, y)`: the row reached by `y`, and the page in that row.
 * When `(x, y)` falls in the gap between two pages, takes the nearer page.
 * Clamps above the first row and below the last. Null for a document without pages.
 */
export function pageNear(layout: Layout, y: number, x?: number, width?: number): number | null {
  const rowCount = layout.rows.length;
  if (rowCount === 0) return null;
  const rowIndex = Math.min(firstRowBelow(layout.rows, y), rowCount - 1);
  let row = layout.rows[rowIndex]!;
  const previous = layout.rows[rowIndex - 1];
  if (y < row.top && previous && row.top - y > y - (previous.top + previous.height)) {
    row = previous;
  }
  if (row.pageCount <= 1 || x === undefined || width === undefined) {
    return row.firstPageIndex;
  }
  const leftIdx = row.firstPageIndex;
  const rightIdx = row.firstPageIndex + 1;
  const boxL = layout.boxes[leftIdx]!;
  const boxR = layout.boxes[rightIdx]!;
  const leftL = pageLeft(boxL, width);
  const rightL = leftL + boxL.width;
  const leftR = pageLeft(boxR, width);

  if (x <= rightL) return leftIdx;
  if (x >= leftR) return rightIdx;
  return x - rightL <= leftR - x ? leftIdx : rightIdx;
}

/** A page-space rectangle (a link's area) as a CSS pixel rectangle inside the page's box. */
export function rectToBox(
  rect: { x0: number; y0: number; x1: number; y1: number },
  page: PageSize,
  rotation: Rotation,
  box: Pick<PageBox, "width" | "height">,
): { left: number; top: number; width: number; height: number } {
  const a = pageToBox({ x: rect.x0, y: rect.y0 }, page, rotation, box);
  const b = pageToBox({ x: rect.x1, y: rect.y1 }, page, rotation, box);
  return {
    left: Math.min(a.x, b.x),
    top: Math.min(a.y, b.y),
    width: Math.abs(b.x - a.x),
    height: Math.abs(b.y - a.y),
  };
}

/**
 * The 1-based page being read: the first page of the row at 30% of the viewport height, or
 * `preferredPage` if it is in that row (e.g. after jumping to the right-hand page), or the
 * last page at the end of the document.
 */
export function currentPageAt(
  layout: Layout,
  viewport: Pick<Viewport, "top" | "height">,
  preferredPage?: number,
): number {
  const count = layout.boxes.length;
  if (count === 0) return 1;
  if (viewport.top + viewport.height >= layout.totalHeight - 1 && viewport.top > 0) {
    if (preferredPage !== undefined && preferredPage >= 1 && preferredPage <= count) {
      const prefRow = rowForPage(layout, preferredPage - 1);
      if (prefRow === layout.rows.length - 1) return preferredPage;
    }
    return count;
  }
  const rowIndex = currentRowAt(layout, viewport);
  const row = layout.rows[rowIndex]!;
  if (preferredPage !== undefined && preferredPage >= 1 && preferredPage <= count) {
    const prefIndex = preferredPage - 1;
    if (prefIndex >= row.firstPageIndex && prefIndex < row.firstPageIndex + row.pageCount) {
      return preferredPage;
    }
  }
  return row.firstPageIndex + 1;
}

/** Scroll position that puts a 1-based page (its row) at the top of the viewport. */
export function scrollTopFor(layout: Layout, page: number): number {
  if (page < 1 || layout.boxes.length === 0) return 0;
  const pageIndex = Math.min(page - 1, layout.boxes.length - 1);
  const rowIndex = rowForPage(layout, pageIndex);
  const row = layout.rows[rowIndex];
  return row ? Math.max(row.top - PAGE_PADDING_PX / 2, 0) : 0;
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
