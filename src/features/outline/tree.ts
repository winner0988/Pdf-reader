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

/** 0-based index of item among its siblings (items sharing the same parent and depth). */
export function siblingIndex(items: OutlineItem[], index: number): number {
  const parent = parentOf(items, index);
  const depth = items[index]!.depth;
  let count = 0;
  for (let i = 0; i < index; i++) {
    if (parentOf(items, i) === parent && items[i]!.depth === depth) {
      count++;
    }
  }
  return count;
}

/** Index of the previous sibling, or -1 if `index` is the first child. */
export function prevSibling(items: OutlineItem[], index: number): number {
  const parent = parentOf(items, index);
  const depth = items[index]!.depth;
  for (let i = index - 1; i >= 0; i--) {
    if (items[i]!.depth < depth) return -1;
    if (items[i]!.depth === depth && parentOf(items, i) === parent) return i;
  }
  return -1;
}

/** Index of the next sibling, or -1 if `index` is the last child. */
export function nextSibling(items: OutlineItem[], index: number): number {
  const parent = parentOf(items, index);
  const depth = items[index]!.depth;
  for (let i = index + 1; i < items.length; i++) {
    if (items[i]!.depth < depth) return -1;
    if (items[i]!.depth === depth && parentOf(items, i) === parent) return i;
  }
  return -1;
}

/** Number of immediate children under `index`. */
export function childrenCount(items: OutlineItem[], index: number): number {
  let count = 0;
  for (let i = index + 1; i < items.length; i++) {
    if (items[i]!.depth <= items[index]!.depth) break;
    if (parentOf(items, i) === index) count++;
  }
  return count;
}

export function canMoveUp(items: OutlineItem[], index: number): boolean {
  return prevSibling(items, index) >= 0;
}

export function canMoveDown(items: OutlineItem[], index: number): boolean {
  return nextSibling(items, index) >= 0;
}

export function canPromote(items: OutlineItem[], index: number): boolean {
  return items[index]!.depth > 0;
}

export function canDemote(items: OutlineItem[], index: number): boolean {
  return prevSibling(items, index) >= 0;
}

export type MoveDirection = "up" | "down" | "promote" | "demote";

/**
 * Computes destination `(id, parent, index)` for moving an outline item (B3-17).
 * Reuses the same "move to parent's index-th child" model for up, down, promote, demote.
 */
export function getMoveParams(
  items: OutlineItem[],
  index: number,
  direction: MoveDirection,
): { id: number; parent: number | null; index: number } | null {
  const item = items[index];
  if (!item) return null;
  const currentParentIdx = parentOf(items, index);
  const currentParentId = currentParentIdx >= 0 ? items[currentParentIdx]!.id : null;
  const currentSibIdx = siblingIndex(items, index);

  switch (direction) {
    case "up": {
      if (!canMoveUp(items, index)) return null;
      return { id: item.id, parent: currentParentId, index: currentSibIdx - 1 };
    }
    case "down": {
      if (!canMoveDown(items, index)) return null;
      return { id: item.id, parent: currentParentId, index: currentSibIdx + 1 };
    }
    case "promote": {
      if (!canPromote(items, index)) return null;
      const grandParentIdx = parentOf(items, currentParentIdx);
      const grandParentId = grandParentIdx >= 0 ? items[grandParentIdx]!.id : null;
      const parentSibIdx = siblingIndex(items, currentParentIdx);
      return { id: item.id, parent: grandParentId, index: parentSibIdx + 1 };
    }
    case "demote": {
      if (!canDemote(items, index)) return null;
      const prev = prevSibling(items, index);
      const prevItem = items[prev]!;
      const count = childrenCount(items, prev);
      return { id: item.id, parent: prevItem.id, index: count };
    }
  }
}
