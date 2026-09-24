// Where each page's thumbnail sits in the sidebar (MVP-18). Thumbnails share one width; their
// height follows the page's shape, capped so that a very tall page does not take the whole list.

import type { PageSize } from "@/features/shell/model";

/** Thumbnail width in CSS pixels. */
export const THUMB_WIDTH = 128;
/** A thumbnail is at most this tall (a strip-shaped page is shown narrower). */
export const MAX_THUMB_HEIGHT = 2 * THUMB_WIDTH;
/** The page number under each thumbnail, and the space between items. */
export const LABEL_HEIGHT = 20;
export const ITEM_GAP = 12;
/** Thumbnails kept rendered above and below the visible ones. */
export const OVERSCAN_ITEMS = 3;

export type ThumbBox = { top: number; width: number; height: number };
export type ThumbLayout = { boxes: ThumbBox[]; totalHeight: number };

/** The thumbnail's size for a page: `THUMB_WIDTH` wide, or less for a very tall page. */
export function thumbSize(page: PageSize): { width: number; height: number } {
  const height = (THUMB_WIDTH * page.heightPt) / page.widthPt;
  if (height <= MAX_THUMB_HEIGHT) return { width: THUMB_WIDTH, height };
  return { width: (MAX_THUMB_HEIGHT * page.widthPt) / page.heightPt, height: MAX_THUMB_HEIGHT };
}

export function layoutThumbs(pages: readonly PageSize[]): ThumbLayout {
  const boxes: ThumbBox[] = [];
  let top = ITEM_GAP;
  for (const page of pages) {
    const size = thumbSize(page);
    boxes.push({ top, ...size });
    top += size.height + LABEL_HEIGHT + ITEM_GAP;
  }
  return { boxes, totalHeight: top };
}

/** The first item whose bottom is below `y` (binary search: a document may have 100,000 pages). */
function firstEndingBelow(boxes: readonly ThumbBox[], y: number): number {
  let low = 0;
  let high = boxes.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    const box = boxes[middle]!;
    if (box.top + box.height + LABEL_HEIGHT < y) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** Items (0-based, inclusive) to keep rendered for a view from `top`, `height` tall. */
export function visibleThumbs(layout: ThumbLayout, top: number, height: number): { first: number; last: number } {
  const count = layout.boxes.length;
  if (count === 0) return { first: 0, last: -1 };
  const first = Math.min(firstEndingBelow(layout.boxes, top), count - 1);
  let last = first;
  while (last + 1 < count && layout.boxes[last + 1]!.top < top + height) last++;
  return { first: Math.max(0, first - OVERSCAN_ITEMS), last: Math.min(count - 1, last + OVERSCAN_ITEMS) };
}

/** A scroll position that shows item `index` whole, moving as little as possible. */
export function scrollToShow(layout: ThumbLayout, index: number, top: number, height: number): number {
  const box = layout.boxes[index];
  if (!box) return top;
  const start = box.top - ITEM_GAP;
  const end = box.top + box.height + LABEL_HEIGHT + ITEM_GAP;
  if (start < top) return Math.max(0, start);
  if (end > top + height) return Math.max(0, end - height);
  return top;
}
