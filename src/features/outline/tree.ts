// The outline as a tree over the flat, pre-order list the main process sends (MVP-09).
// Items are identified by their index in that list; `depth` 0 is the top level.

import type { OutlineItem } from "@/ipc/generated/contract";

/** What the sidebar shows for the outline. */
export type OutlineView =
  | { status: "none" }
  | { status: "loading" }
  | { status: "failed" }
  | { status: "ready"; items: OutlineItem[]; truncated: boolean };

export function hasChildren(items: OutlineItem[], index: number): boolean {
  const next = items[index + 1];
  return next !== undefined && next.depth > items[index]!.depth;
}

/** Index of the item's parent, or -1 at the top level. */
export function parentOf(items: OutlineItem[], index: number): number {
  const depth = items[index]!.depth;
  for (let before = index - 1; before >= 0; before--) {
    if (items[before]!.depth < depth) return before;
  }
  return -1;
}

/** Items shown when only the items in `expanded` have their children visible. */
export function visibleItems(items: OutlineItem[], expanded: ReadonlySet<number>): number[] {
  const visible: number[] = [];
  let hiddenBelow = Infinity; // depth under a collapsed item
  items.forEach((item, index) => {
    if (item.depth > hiddenBelow) return;
    hiddenBelow = Infinity;
    visible.push(index);
    if (hasChildren(items, index) && !expanded.has(index)) hiddenBelow = item.depth;
  });
  return visible;
}

/** Initially expanded: the top level, so the first two levels show. */
export function initiallyExpanded(items: OutlineItem[]): Set<number> {
  const expanded = new Set<number>();
  items.forEach((item, index) => {
    if (item.depth === 0 && hasChildren(items, index)) expanded.add(index);
  });
  return expanded;
}

/** 0-based page an item jumps to, if it jumps anywhere in this document. */
export function pageOf(item: OutlineItem): number | null {
  return item.target?.kind === "page" ? item.target.pageIndex : null;
}

/**
 * The item for the page being read (1-based): the last item, in reading order, whose page is at
 * or before it. -1 when there is none.
 */
export function currentItem(items: OutlineItem[], currentPage: number): number {
  let found = -1;
  items.forEach((item, index) => {
    const page = pageOf(item);
    if (page !== null && page + 1 <= currentPage) found = index;
  });
  return found;
}

/** The visible item that stands for `index`: itself, or its closest visible ancestor. */
export function visibleAncestor(items: OutlineItem[], visible: readonly number[], index: number): number {
  let at = index;
  while (at >= 0 && !visible.includes(at)) at = parentOf(items, at);
  return at;
}
