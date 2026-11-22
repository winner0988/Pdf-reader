// Pure functions for document watermark (B6-06, #591).

import { hexToRgb, type WatermarkForm } from "@/features/batch/watermark";
import { parsePageRanges } from "@/features/saving/pageMarks";
import type { Edit, PageSize, StampImageId, WatermarkPlace } from "@/ipc/generated/contract";

export type PageChoice = "all" | "first" | "allButFirst" | "range";

/**
 * Parses user page choice into a sorted array of 0-based page indices.
 * Returns null if the range is invalid or empty.
 */
export function pagesOf(
  choice: PageChoice,
  rangeText: string,
  count: number,
): number[] | null {
  if (count <= 0) return null;
  switch (choice) {
    case "all":
      return Array.from({ length: count }, (_, i) => i);
    case "first":
      return [0];
    case "allButFirst":
      return count > 1 ? Array.from({ length: count - 1 }, (_, i) => i + 1) : [];
    case "range": {
      const parsed = parsePageRanges(rangeText, count, count);
      return parsed && parsed.length > 0 ? parsed : null;
    }
  }
}

/** How much of the page a watermark may take across or diagonally. */
const FIT = 0.9;
/** How far from the top or bottom of the page a watermark is, in points. */
const MARGIN_PT = 36.0;

export type WatermarkPlacement = {
  center: { x: number; y: number };
  angle: number; // in radians
  scale: number;
};

/**
 * Calculates watermark placement matching worker's formula (watermark.rs).
 * Coordinates are in page space (origin at top-left, y pointing down).
 */
export function placement(
  page: PageSize,
  mark: { width: number; height: number },
  diagonal: boolean,
  place: WatermarkPlace,
): WatermarkPlacement {
  const pageWidth = page.widthPt;
  const pageHeight = page.heightPt;
  if (!(pageWidth > 0 && pageHeight > 0) || !(mark.width > 0 && mark.height > 0)) {
    return { center: { x: 0, y: 0 }, angle: 0, scale: 1 };
  }

  const angle = diagonal ? Math.atan2(pageHeight, pageWidth) : 0.0;
  const room = (diagonal ? Math.hypot(pageWidth, pageHeight) : pageWidth) * FIT;
  const scale = Math.min(room / mark.width, 1.0);
  const reach =
    (mark.width * scale / 2.0) * Math.sin(angle) +
    (mark.height * scale / 2.0) * Math.cos(angle);
  const x = pageWidth / 2.0;
  let y = pageHeight / 2.0;
  if (place === "top") {
    y = MARGIN_PT + reach;
  } else if (place === "bottom") {
    y = pageHeight - MARGIN_PT - reach;
  }

  return { center: { x, y }, angle, scale };
}

/**
 * Converts form state, target page list, and optional image into an Edit::AddWatermark payload.
 */
export function editOf(
  form: WatermarkForm,
  pages: number[],
  image: StampImageId | null,
): Edit {
  return {
    kind: "addWatermark",
    pages,
    watermark: {
      content:
        form.type === "text"
          ? {
              kind: "text",
              text: form.text,
              size: form.size,
              color: hexToRgb(form.color),
            }
          : {
              kind: "picture",
              widthPt: form.pictureWidth,
            },
      opacity: form.opacity,
      diagonal: form.diagonal,
      place: form.place,
      pages: "all",
    },
    image: form.type === "picture" ? image : null,
  };
}
