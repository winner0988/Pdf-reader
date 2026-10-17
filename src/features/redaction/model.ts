// Security redaction model (B2-21, ADR 0016): geometry of marks from text selection,
// grouping marks by page, and default rect placement.

import type { PageSize } from "@/features/shell/model";
import { LIMITS, type PageAnnotation, type Point, type Quad, type Rect } from "@/ipc/generated/contract";

export type RedactionMarkItem = {
  page: number; // 0-based page index
  annotation: PageAnnotation;
};

export type PageRedactionGroup = {
  page: number; // 0-based page index
  marks: PageAnnotation[];
};

export const REDACTION_WIDTH_PT = 140;
export const REDACTION_HEIGHT_PT = 24;

/** Computes the bounding rectangle of a Quad in page space with minimum side limits. */
export function rectOfQuad(quad: Quad): Rect {
  const x0 = Math.min(quad.ul.x, quad.ur.x, quad.ll.x, quad.lr.x);
  const x1 = Math.max(quad.ul.x, quad.ur.x, quad.ll.x, quad.lr.x);
  const y0 = Math.min(quad.ul.y, quad.ur.y, quad.ll.y, quad.lr.y);
  const y1 = Math.max(quad.ul.y, quad.ur.y, quad.ll.y, quad.lr.y);
  const minSide = LIMITS.minAnnotationSidePt;
  const width = Math.max(x1 - x0, minSide);
  const height = Math.max(y1 - y0, minSide);
  const midX = (x0 + x1) / 2;
  const midY = (y0 + y1) / 2;
  return {
    x0: Math.round((midX - width / 2) * 100) / 100,
    y0: Math.round((midY - height / 2) * 100) / 100,
    x1: Math.round((midX + width / 2) * 100) / 100,
    y1: Math.round((midY + height / 2) * 100) / 100,
  };
}

/** Computes a bounded Rect from two dragged points on a page. */
export function rectOfPoints(start: Point, end: Point, minSide = LIMITS.minAnnotationSidePt): Rect {
  const x0 = Math.min(start.x, end.x);
  const x1 = Math.max(start.x, end.x);
  const y0 = Math.min(start.y, end.y);
  const y1 = Math.max(start.y, end.y);
  const width = Math.max(x1 - x0, minSide);
  const height = Math.max(y1 - y0, minSide);
  const midX = (x0 + x1) / 2;
  const midY = (y0 + y1) / 2;
  return {
    x0: Math.round((midX - width / 2) * 100) / 100,
    y0: Math.round((midY - height / 2) * 100) / 100,
    x1: Math.round((midX + width / 2) * 100) / 100,
    y1: Math.round((midY + height / 2) * 100) / 100,
  };
}

/** Default redaction mark placed around `at` on `page`. */
export function defaultRedactionRectAt(at: Point, page: PageSize): Rect {
  const width = Math.min(REDACTION_WIDTH_PT, page.widthPt);
  const height = Math.min(REDACTION_HEIGHT_PT, page.heightPt);
  const place = (center: number, size: number, limit: number): [number, number] => {
    const start = Math.min(Math.max(center - size / 2, 0), Math.max(limit - size, 0));
    return [Math.round(start * 100) / 100, Math.round((start + size) * 100) / 100];
  };
  const [x0, x1] = place(at.x, width, page.widthPt);
  const [y0, y1] = place(at.y, height, page.heightPt);
  return { x0, y0, x1, y1 };
}

/** Groups redaction items by page index in ascending order. */
export function groupMarksByPage(items: readonly RedactionMarkItem[]): PageRedactionGroup[] {
  const map = new Map<number, PageAnnotation[]>();
  for (const item of items) {
    const list = map.get(item.page) ?? [];
    list.push(item.annotation);
    map.set(item.page, list);
  }
  const pages = Array.from(map.keys()).sort((a, b) => a - b);
  return pages.map((page) => ({ page, marks: map.get(page)! }));
}
