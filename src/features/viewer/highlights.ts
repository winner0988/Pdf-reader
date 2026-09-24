// Search hit highlights over the pages (MVP-10). Hits come in page points; they are drawn in
// the page box, so they follow zoom and rotation like the page itself.

import type { PageSize, Rotation } from "@/features/shell/model";
import { pageLeft, pageToBox, type Layout, type PageBox, type Viewport } from "@/features/viewer/layout";
import type { Quad } from "@/ipc/generated/contract";

/** One search hit: a quad per line it spans, in page points. */
export type Highlight = { pageIndex: number; quads: Quad[] };

/** All hits in document order (sorted by page) and the index of the selected one (-1: none). */
export type Highlights = { hits: readonly Highlight[]; current: number };

/** SVG polygons for one page, in page box pixels. */
export type PageOverlay = { all: string[]; current: string[] };

/** The hits on `pageIndex`: `hits.slice(start, end)`. Binary search, as there may be 10,000 hits. */
export function hitRange(hits: readonly Highlight[], pageIndex: number): [start: number, end: number] {
  const firstFrom = (page: number) => {
    let low = 0;
    let high = hits.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (hits[middle]!.pageIndex < page) low = middle + 1;
      else high = middle;
    }
    return low;
  };
  return [firstFrom(pageIndex), firstFrom(pageIndex + 1)];
}

const round = (value: number) => Math.round(value * 100) / 100;

function corners(quad: Quad, page: PageSize, rotation: Rotation, box: PageBox) {
  return [quad.ul, quad.ur, quad.lr, quad.ll].map((point) => pageToBox(point, page, rotation, box));
}

/** The `points` of an SVG polygon for `quad` drawn in `box`. */
export function quadPoints(quad: Quad, page: PageSize, rotation: Rotation, box: PageBox): string {
  return corners(quad, page, rotation, box)
    .map((point) => `${round(point.x)},${round(point.y)}`)
    .join(" ");
}

/** What to draw over page `pageIndex`, or null if it has no hits. */
export function pageOverlay(
  highlights: Highlights,
  pageIndex: number,
  page: PageSize,
  rotation: Rotation,
  box: PageBox,
): PageOverlay | null {
  const [start, end] = hitRange(highlights.hits, pageIndex);
  if (start === end) return null;
  const polygons = (hit: Highlight) => hit.quads.map((quad) => quadPoints(quad, page, rotation, box));
  const { current } = highlights;
  return {
    all: highlights.hits.slice(start, end).flatMap(polygons),
    current: current >= start && current < end ? polygons(highlights.hits[current]!) : [],
  };
}

/**
 * The scroll position that shows `hit` a third of the way down the viewport
 * (docs/ux/screen-map.md, section 3). Scrolls sideways only if the hit is not fully in view.
 */
export function revealScroll(
  hit: Highlight,
  pages: readonly PageSize[],
  rotation: Rotation,
  layout: Layout,
  width: number,
  viewport: Viewport,
): { top: number; left?: number } | null {
  const box = layout.boxes[hit.pageIndex];
  const page = pages[hit.pageIndex];
  if (!box || !page || hit.quads.length === 0) return null;
  const points = hit.quads.flatMap((quad) => corners(quad, page, rotation, box));
  const xs = points.map((point) => point.x);
  const top = box.top + Math.min(...points.map((point) => point.y));
  const left = pageLeft(box, width) + Math.min(...xs);
  const right = pageLeft(box, width) + Math.max(...xs);
  const position: { top: number; left?: number } = { top: Math.max(0, top - viewport.height / 3) };
  if (left < viewport.left || right > viewport.left + viewport.width) {
    position.left = Math.max(0, left - viewport.width / 3);
  }
  return position;
}
