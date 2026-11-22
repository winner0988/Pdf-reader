// Pure functions for cropping pages (B6-04, #589).

import { LIMITS, type CropMargins, type PageSize } from "@/ipc/generated/contract";

export type CropChoice = "selected" | "current" | "all" | "odd" | "even";

/**
 * Returns the 0-based page indices targeted by `choice`.
 * "odd" and "even" refer to 1-based page numbers:
 * - odd pages: 1st, 3rd, 5th... (indices 0, 2, 4...)
 * - even pages: 2nd, 4th, 6th... (indices 1, 3, 5...)
 */
export function pagesFor(
  choice: CropChoice,
  count: number,
  selected: readonly number[],
  current: number,
): number[] {
  if (count <= 0) return [];
  switch (choice) {
    case "selected": {
      const valid = Array.from(new Set(selected.filter((p) => p >= 0 && p < count)));
      valid.sort((a, b) => a - b);
      return valid;
    }
    case "current":
      return current >= 0 && current < count ? [current] : [];
    case "all":
      return Array.from({ length: count }, (_, i) => i);
    case "odd":
      return Array.from({ length: count }, (_, i) => i).filter((i) => i % 2 === 0);
    case "even":
      return Array.from({ length: count }, (_, i) => i).filter((i) => i % 2 === 1);
  }
}

/**
 * Calculates page margins in points from a drag box specified in preview CSS coordinates.
 */
export function marginsFromDrag(
  from: { x: number; y: number },
  to: { x: number; y: number },
  size: PageSize,
  scale: number,
): CropMargins {
  if (scale <= 0) {
    return { top: 0, right: 0, bottom: 0, left: 0 };
  }
  const wPx = size.widthPt * scale;
  const hPx = size.heightPt * scale;
  const xMin = Math.min(from.x, to.x);
  const xMax = Math.max(from.x, to.x);
  const yMin = Math.min(from.y, to.y);
  const yMax = Math.max(from.y, to.y);

  const clampedX0 = Math.max(0, Math.min(wPx, xMin));
  const clampedX1 = Math.max(0, Math.min(wPx, xMax));
  const clampedY0 = Math.max(0, Math.min(hPx, yMin));
  const clampedY1 = Math.max(0, Math.min(hPx, yMax));

  const leftPt = Math.round(clampedX0 / scale);
  const topPt = Math.round(clampedY0 / scale);
  const rightPt = Math.round((wPx - clampedX1) / scale);
  const bottomPt = Math.round((hPx - clampedY1) / scale);

  return {
    top: Math.max(0, topPt),
    right: Math.max(0, rightPt),
    bottom: Math.max(0, bottomPt),
    left: Math.max(0, leftPt),
  };
}

/**
 * Returns the first 0-based page index whose cropped width or height would fall below
 * `LIMITS.minCroppedSidePt`, or null if all chosen pages meet the minimum size.
 */
export function firstTooSmall(
  pages: readonly number[],
  sizes: readonly PageSize[],
  margins: CropMargins,
): number | null {
  for (const page of pages) {
    const size = sizes[page];
    if (!size) continue;
    const remainingW = size.widthPt - margins.left - margins.right;
    const remainingH = size.heightPt - margins.top - margins.bottom;
    if (remainingW < LIMITS.minCroppedSidePt || remainingH < LIMITS.minCroppedSidePt) {
      return page;
    }
  }
  return null;
}

/**
 * Returns true if all 4 margins are zero (nothing to crop).
 */
export function cropsNothing(margins: CropMargins): boolean {
  return margins.top === 0 && margins.right === 0 && margins.bottom === 0 && margins.left === 0;
}
