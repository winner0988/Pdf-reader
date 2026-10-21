import { describe, expect, it } from "vitest";

import {
  canDemote,
  canMoveDown,
  canMoveUp,
  canPromote,
  childrenCount,
  currentItem,
  getMoveParams,
  hasChildren,
  initiallyExpanded,
  nextSibling,
  pageOf,
  parentOf,
  prevSibling,
  siblingIndex,
  visibleAncestor,
  visibleItems,
} from "@/features/outline/tree";
import type { OutlineItem } from "@/ipc/generated/contract";

const page = (pageIndex: number) => ({ kind: "page" as const, pageIndex, x: null, y: null });

// The QA-01 three-level sample: Chapter 1 > Section 1.1 > Subsection 1.1.1, Chapter 2 > 2.1, Appendix.
const items: OutlineItem[] = [
  { id: 10, title: "Chapter 1", depth: 0, target: page(0) },
  { id: 11, title: "Section 1.1", depth: 1, target: page(1) },
  { id: 12, title: "Subsection 1.1.1", depth: 2, target: page(2) },
  { id: 13, title: "Chapter 2", depth: 0, target: page(3) },
  { id: 14, title: "Section 2.1", depth: 1, target: page(4) },
  { id: 15, title: "Appendix", depth: 0, target: page(5) },
];

describe("outline tree", () => {
  it("knows parents and children", () => {
    expect([0, 1, 2, 3, 4, 5].map((index) => hasChildren(items, index))).toEqual([true, true, false, true, false, false]);
    expect([0, 1, 2, 3, 4, 5].map((index) => parentOf(items, index))).toEqual([-1, 0, 1, -1, 3, -1]);
  });

  it("shows the first two levels at first", () => {
    const expanded = initiallyExpanded(items);
    expect([...expanded]).toEqual([0, 3]);
    expect(visibleItems(items, expanded)).toEqual([0, 1, 3, 4, 5]);
  });

  it("hides everything under a collapsed item", () => {
    expect(visibleItems(items, new Set())).toEqual([0, 3, 5]);
    expect(visibleItems(items, new Set([0, 1]))).toEqual([0, 1, 2, 3, 5]);
    // A collapsed child hides its own children even under an expanded parent.
    expect(visibleItems(items, new Set([0]))).toEqual([0, 1, 3, 5]);
  });

  it("finds the item for the current page", () => {
    expect(currentItem(items, 1)).toBe(0);
    expect(currentItem(items, 3)).toBe(2);
    expect(currentItem(items, 5)).toBe(4);
    expect(currentItem(items, 99)).toBe(5);
    expect(currentItem([{ id: 99, title: "Later", depth: 0, target: page(4) }], 1)).toBe(-1);
  });

  it("marks the closest visible ancestor when the current item is hidden", () => {
    const visible = visibleItems(items, new Set());
    expect(visibleAncestor(items, visible, 2)).toBe(0);
    expect(visibleAncestor(items, visible, 4)).toBe(3);
    expect(visibleAncestor(items, visible, -1)).toBe(-1);
  });

  it("only page targets jump", () => {
    expect(pageOf(items[1]!)).toBe(1);
    expect(pageOf({ id: 1, title: "Web", depth: 0, target: { kind: "uri", uri: "https://example.invalid/" } })).toBeNull();
    expect(pageOf({ id: 2, title: "Run", depth: 0, target: { kind: "blocked", action: "launch", target: "calc.exe" } })).toBeNull();
    expect(pageOf({ id: 3, title: "None", depth: 0, target: null })).toBeNull();
  });

  it("handles outline item navigation and moving", () => {
    // Siblings of top level: [0, 3, 5]
    expect(siblingIndex(items, 0)).toBe(0);
    expect(siblingIndex(items, 3)).toBe(1);
    expect(siblingIndex(items, 5)).toBe(2);
    // Prev / Next siblings
    expect(prevSibling(items, 0)).toBe(-1);
    expect(nextSibling(items, 0)).toBe(3);
    expect(prevSibling(items, 3)).toBe(0);
    expect(nextSibling(items, 3)).toBe(5);
    expect(nextSibling(items, 5)).toBe(-1);

    // Section 1.1 (index 1) is the only child of Chapter 1
    expect(childrenCount(items, 0)).toBe(1);
    expect(canMoveUp(items, 1)).toBe(false);
    expect(canMoveDown(items, 1)).toBe(false);
    expect(canPromote(items, 1)).toBe(true);
    expect(canDemote(items, 1)).toBe(false);

    // Move up / down top-level items
    expect(canMoveUp(items, 0)).toBe(false);
    expect(canMoveDown(items, 0)).toBe(true);
    expect(getMoveParams(items, 0, "down")).toEqual({ id: 10, parent: null, index: 1 });
    expect(getMoveParams(items, 3, "up")).toEqual({ id: 13, parent: null, index: 0 });

    // Promote Section 1.1 (index 1) to top level after Chapter 1
    expect(getMoveParams(items, 1, "promote")).toEqual({ id: 11, parent: null, index: 1 });

    // Demote Chapter 2 (index 3) under Chapter 1 (index 0)
    expect(canDemote(items, 3)).toBe(true);
    expect(getMoveParams(items, 3, "demote")).toEqual({ id: 13, parent: 10, index: 1 });
  });
});
