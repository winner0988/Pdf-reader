import { describe, expect, it } from "vitest";

import {
  extendTo,
  movedTo,
  movesNothing,
  NO_SELECTION,
  selectAll,
  selectionAfter,
  selectOnly,
  toggle,
  within,
} from "@/features/thumbnails/selection";

describe("thumbnail selection", () => {
  it("selects one, toggles with Ctrl and extends with Shift from the last one picked", () => {
    let selection = selectOnly(2);
    selection = toggle(selection, 6);
    expect(selection).toEqual({ pages: [2, 6], anchor: 6 });
    // Shift extends from the anchor, whichever way.
    expect(extendTo(selection, 4).pages).toEqual([4, 5, 6]);
    expect(extendTo(selection, 8).pages).toEqual([6, 7, 8]);
    expect(toggle(selection, 2)).toEqual({ pages: [6], anchor: 2 });
    expect(extendTo(NO_SELECTION, 3)).toEqual(selectOnly(3));
    expect(selectAll(3)).toEqual({ pages: [0, 1, 2], anchor: 0 });
  });

  it("drops pages a document no longer has", () => {
    const selection = { pages: [1, 4, 7], anchor: 7 };
    expect(within(selection, 8)).toBe(selection);
    expect(within(selection, 5)).toEqual({ pages: [1, 4], anchor: null });
  });

  it("follows the pages an edit changed", () => {
    expect(selectionAfter({ kind: "rotatePages", pages: [5, 1], by: "cw90" }).pages).toEqual([1, 5]);
    // Pages 2 and 6 moved before page 9 of ten: they are now the 7th and 8th.
    expect(selectionAfter({ kind: "movePages", pages: [5, 1], before: 8 }).pages).toEqual([6, 7]);
    expect(selectionAfter({ kind: "insertBlankPage", at: 3, like: 2 })).toEqual(selectOnly(3));
    expect(selectionAfter({ kind: "deletePages", pages: [0] })).toEqual(NO_SELECTION);
  });

  it("knows a move that changes nothing", () => {
    expect(movedTo([4], 0)).toEqual([0]);
    expect(movesNothing([3, 4], 3)).toBe(true);
    expect(movesNothing([3, 4], 5)).toBe(true);
    expect(movesNothing([3, 4], 6)).toBe(false);
    expect(movesNothing([3, 5], 5)).toBe(false);
  });
});
