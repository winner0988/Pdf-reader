// Which page thumbnails are selected (B2-05), and where the selected pages are after an edit so
// the selection can follow them. Pages are 0-based.

import type { Edit } from "@/ipc/generated/contract";

/** Selected pages, ascending, and the page a Shift+click or Shift+arrow extends from. */
export type Selection = { pages: number[]; anchor: number | null };

export const NO_SELECTION: Selection = { pages: [], anchor: null };

/** Only `page`, which becomes the anchor. */
export function selectOnly(page: number): Selection {
  return { pages: [page], anchor: page };
}

/** `page` added to the selection, or taken out of it; it becomes the anchor. */
export function toggle(selection: Selection, page: number): Selection {
  const pages = selection.pages.includes(page)
    ? selection.pages.filter((selected) => selected !== page)
    : [...selection.pages, page].sort((a, b) => a - b);
  return { pages, anchor: page };
}

/** Every page from the anchor to `page`, both included; only `page` without an anchor. */
export function extendTo(selection: Selection, page: number): Selection {
  if (selection.anchor === null) return selectOnly(page);
  const [from, to] = selection.anchor <= page ? [selection.anchor, page] : [page, selection.anchor];
  return { pages: range(from, to + 1), anchor: selection.anchor };
}

/** All `count` pages. */
export function selectAll(count: number): Selection {
  return { pages: range(0, count), anchor: count > 0 ? 0 : null };
}

/** The selection within a document of `count` pages: pages beyond it are dropped. */
export function within(selection: Selection, count: number): Selection {
  if (selection.pages.every((page) => page < count) && (selection.anchor ?? 0) < Math.max(count, 1)) {
    return selection;
  }
  const pages = selection.pages.filter((page) => page < count);
  return { pages, anchor: selection.anchor !== null && selection.anchor < count ? selection.anchor : null };
}

/** Where `pages` go when moved just before page `before` (`Edit.movePages`): together, in order. */
export function movedTo(pages: readonly number[], before: number): number[] {
  const start = before - pages.filter((page) => page < before).length;
  return range(start, start + pages.length);
}

/** Whether moving `pages` just before page `before` would leave every page where it is. */
export function movesNothing(pages: readonly number[], before: number): boolean {
  const sorted = [...pages].sort((a, b) => a - b);
  return movedTo(sorted, before).every((page, index) => page === sorted[index]);
}

/**
 * What is selected once `edit` is applied: the same pages turned, the pages moved where they went,
 * the page inserted; nothing once pages are deleted.
 */
export function selectionAfter(edit: Edit): Selection {
  switch (edit.kind) {
    case "rotatePages": {
      const pages = [...edit.pages].sort((a, b) => a - b);
      return { pages, anchor: pages[0] ?? null };
    }
    case "movePages": {
      const pages = movedTo(edit.pages, edit.before);
      return { pages, anchor: pages[0] ?? null };
    }
    case "insertBlankPage":
      return selectOnly(edit.at);
    case "deletePages":
      return NO_SELECTION;
  }
}

function range(from: number, to: number): number[] {
  return Array.from({ length: Math.max(to - from, 0) }, (_, index) => from + index);
}
