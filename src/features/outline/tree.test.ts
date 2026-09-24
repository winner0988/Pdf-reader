import { describe, expect, it } from "vitest";

import {
  currentItem,
  hasChildren,
  initiallyExpanded,
  pageOf,
  parentOf,
  visibleAncestor,
  visibleItems,
} from "@/features/outline/tree";
import type { OutlineItem } from "@/ipc/generated/contract";

const page = (pageIndex: number) => ({ kind: "page" as const, pageIndex, x: null, y: null });

// The QA-01 three-level sample: Chapter 1 > Section 1.1 > Subsection 1.1.1, Chapter 2 > 2.1, Appendix.
const items: OutlineItem[] = [
  { title: "Chapter 1", depth: 0, target: page(0) },
  { title: "Section 1.1", depth: 1, target: page(1) },
  { title: "Subsection 1.1.1", depth: 2, target: page(2) },
  { title: "Chapter 2", depth: 0, target: page(3) },
  { title: "Section 2.1", depth: 1, target: page(4) },
  { title: "Appendix", depth: 0, target: page(5) },
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
    expect(currentItem([{ title: "Later", depth: 0, target: page(4) }], 1)).toBe(-1);
  });

  it("marks the closest visible ancestor when the current item is hidden", () => {
    const visible = visibleItems(items, new Set());
    expect(visibleAncestor(items, visible, 2)).toBe(0);
    expect(visibleAncestor(items, visible, 4)).toBe(3);
    expect(visibleAncestor(items, visible, -1)).toBe(-1);
  });

  it("only page targets jump", () => {
    expect(pageOf(items[1]!)).toBe(1);
    expect(pageOf({ title: "Web", depth: 0, target: { kind: "uri", uri: "https://example.invalid/" } })).toBeNull();
    expect(pageOf({ title: "Run", depth: 0, target: { kind: "blocked", action: "launch", target: "calc.exe" } })).toBeNull();
    expect(pageOf({ title: "None", depth: 0, target: null })).toBeNull();
  });
});
