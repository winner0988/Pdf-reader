// The pen and the stamps (B2-08, docs/architecture/annotations.md): what the user can choose,
// and the geometry of drawing, placing, moving and resizing. Pure, so that it is tested without
// a page.

import type { PageSize, Rotation } from "@/features/shell/model";
import { boxToPage, type PageBox } from "@/features/viewer/layout";
import { LIMITS, type InkColor, type InkWidth, type Point, type Rect, type StampName } from "@/ipc/generated/contract";

/** What the pointer does on the pages: draws with the pen, or places a stamp. */
export type Tool = { kind: "pen"; color: InkColor; width: InkWidth } | { kind: "stamp"; stamp: StampName };

export const INK_COLORS: InkColor[] = ["black", "red", "blue", "green"];
export const INK_WIDTHS: InkWidth[] = ["thin", "medium", "thick"];
export const STAMPS: StampName[] = [
  "approved",
  "notApproved",
  "draft",
  "final",
  "confidential",
  "forComment",
  "asIs",
  "topSecret",
];

/** The pen colors as the worker draws them (docs/architecture/annotations.md). */
export const INK_SWATCH: Record<InkColor, string> = {
  black: "rgb(0 0 0)",
  red: "rgb(217 26 26)",
  blue: "rgb(26 77 217)",
  green: "rgb(26 153 51)",
};

/** How thick each pen width is, in points. */
export const INK_WIDTH_PT: Record<InkWidth, number> = { thin: 1, medium: 2.5, thick: 5 };

/** A stamp is put down 150 points wide, in the shape MuPDF draws them (190 : 50). */
export const STAMP_WIDTH_PT = 150;
export const STAMP_HEIGHT_PT = (STAMP_WIDTH_PT * 50) / 190;

/** The least a pen's points are apart on the page, in points: closer ones add nothing. */
export const MIN_POINT_DISTANCE_PT = 1;

/**
 * `points` thinned: the first is kept, then every one at least `distance` from the one kept
 * before it, and the last, so that the stroke ends where the pen did.
 */
export function thin(points: readonly Point[], distance: number): Point[] {
  const kept: Point[] = [];
  for (const point of points) {
    const last = kept.at(-1);
    if (!last || Math.hypot(point.x - last.x, point.y - last.y) >= distance) kept.push(point);
  }
  // The stroke ends where the pen did, unless that is where the last kept point is already.
  const end = points.at(-1);
  const last = kept.at(-1);
  if (end && last && (end.x !== last.x || end.y !== last.y)) kept.push(end);
  return kept;
}

/**
 * The points of a stroke as the main process takes them: thinned, and not more than a drawing may
 * have, in hundredths of a point (more digits say nothing and only make the edit longer).
 */
export function strokePoints(points: readonly Point[]): Point[] {
  const hundredths = (value: number) => Math.round(value * 100) / 100;
  let distance = MIN_POINT_DISTANCE_PT;
  let kept = thin(points, distance);
  while (kept.length > LIMITS.maxInkPoints) {
    distance *= 2;
    kept = thin(points, distance);
  }
  return kept.map((point) => ({ x: hundredths(point.x), y: hundredths(point.y) }));
}

/** `point` (page space) as it is, or on the page's edge nearest to it when the pointer left the page. */
export function keptPointOnPage(point: Point, page: PageSize): Point {
  return { x: Math.min(Math.max(point.x, 0), page.widthPt), y: Math.min(Math.max(point.y, 0), page.heightPt) };
}

/** The rectangle a stamp takes when put down at `at` (page space), inside the page. */
export function stampRectAt(at: Point, page: PageSize): Rect {
  const place = (center: number, size: number, limit: number): [number, number] => {
    const start = Math.min(Math.max(center - size / 2, 0), Math.max(limit - size, 0));
    return [start, start + size];
  };
  const [x0, x1] = place(at.x, STAMP_WIDTH_PT, page.widthPt);
  const [y0, y1] = place(at.y, STAMP_HEIGHT_PT, page.heightPt);
  return { x0, y0, x1, y1 };
}

/** A box on a page as the screen shows it, in CSS pixels from the page's top left. */
export type Box = { left: number; top: number; width: number; height: number };

/** The places on a box that can be dragged: its corners and the middles of its sides. */
export type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

export const HANDLES: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/** The corners of a box: the ones a stamp is resized by (it keeps its shape). */
export const isCorner = (handle: Handle) => handle.length === 2;

/** `box` moved by `dx` and `dy`. */
export function moved(box: Box, dx: number, dy: number): Box {
  return { ...box, left: box.left + dx, top: box.top + dy };
}

/**
 * `box` with the side or corner `handle` dragged by `dx` and `dy`, never less than `min` wide or
 * high. With `keepShape` (a corner of a stamp) the box keeps its shape, and the opposite corner
 * stays where it is.
 */
export function dragged(box: Box, handle: Handle, dx: number, dy: number, min: number, keepShape: boolean): Box {
  const west = handle.includes("w");
  const east = handle.includes("e");
  const north = handle.includes("n");
  const south = handle.includes("s");
  if (keepShape && isCorner(handle)) {
    // How far the corner went from the opposite one, as a fraction of each side.
    const along = (east ? dx : -dx) / box.width;
    const down = (south ? dy : -dy) / box.height;
    const change = Math.abs(along) >= Math.abs(down) ? along : down;
    const scale = Math.max(1 + change, min / Math.min(box.width, box.height));
    const width = box.width * scale;
    const height = box.height * scale;
    return {
      left: west ? box.left + box.width - width : box.left,
      top: north ? box.top + box.height - height : box.top,
      width,
      height,
    };
  }
  let left = box.left;
  let right = box.left + box.width;
  let top = box.top;
  let bottom = box.top + box.height;
  if (west) left = Math.min(left + dx, right - min);
  if (east) right = Math.max(right + dx, left + min);
  if (north) top = Math.min(top + dy, bottom - min);
  if (south) bottom = Math.max(bottom + dy, top + min);
  return { left, top, width: right - left, height: bottom - top };
}

/** The rectangle (page space) of a box on a page as the screen shows it, whatever its rotation. */
export function rectOfBox(box: Box, page: PageSize, rotation: Rotation, size: PageBox): Rect {
  const a = boxToPage({ x: box.left, y: box.top }, page, rotation, size);
  const b = boxToPage({ x: box.left + box.width, y: box.top + box.height }, page, rotation, size);
  return { x0: Math.min(a.x, b.x), y0: Math.min(a.y, b.y), x1: Math.max(a.x, b.x), y1: Math.max(a.y, b.y) };
}

/** `rect` (page space) moved, if it can be, so that it lies inside `page`; as it was if it is larger. */
export function keptOnPage(rect: Rect, page: PageSize): Rect {
  const shift = (from: number, to: number, limit: number) => {
    if (to - from > limit) return 0;
    if (from < 0) return -from;
    if (to > limit) return limit - to;
    return 0;
  };
  const dx = shift(rect.x0, rect.x1, page.widthPt);
  const dy = shift(rect.y0, rect.y1, page.heightPt);
  return { x0: rect.x0 + dx, y0: rect.y0 + dy, x1: rect.x1 + dx, y1: rect.y1 + dy };
}
