import { describe, expect, it } from "vitest";

import {
  adjustHistoryForEdit,
  canGoBack,
  canGoForward,
  clampHistoryToPageCount,
  createNavigationHistory,
  historyGoBack,
  historyGoForward,
  MAX_HISTORY_ENTRIES,
  recordNavigation,
  type NavigationLocation,
} from "./history";

const loc = (page: number, scrollTop = 0): NavigationLocation => ({
  page,
  scrollTop,
  zoom: "fitWidth",
});

describe("navigation history model (B4-01)", () => {
  it("starts empty with nowhere to go", () => {
    const history = createNavigationHistory();
    expect(canGoBack(history)).toBe(false);
    expect(canGoForward(history)).toBe(false);
    expect(history.back).toHaveLength(0);
    expect(history.forward).toHaveLength(0);
  });

  it("records location before a jump and allows going back", () => {
    let history = createNavigationHistory();
    history = recordNavigation(history, loc(1, 100));

    expect(canGoBack(history)).toBe(true);
    expect(canGoForward(history)).toBe(false);
    expect(history.back).toEqual([loc(1, 100)]);

    const result = historyGoBack(history, loc(5, 500));
    expect(result).not.toBeNull();
    expect(result!.target).toEqual(loc(1, 100));
    expect(canGoBack(result!.nextHistory)).toBe(false);
    expect(canGoForward(result!.nextHistory)).toBe(true);
    expect(result!.nextHistory.forward).toEqual([loc(5, 500)]);
  });

  it("allows going forward after going back", () => {
    let history = createNavigationHistory();
    history = recordNavigation(history, loc(1, 100));

    const backResult = historyGoBack(history, loc(5, 500));
    expect(backResult).not.toBeNull();
    history = backResult!.nextHistory;

    const fwdResult = historyGoForward(history, loc(1, 150));
    expect(fwdResult).not.toBeNull();
    expect(fwdResult!.target).toEqual(loc(5, 500));
    expect(canGoBack(fwdResult!.nextHistory)).toBe(true);
    expect(canGoForward(fwdResult!.nextHistory)).toBe(false);
    expect(fwdResult!.nextHistory.back).toEqual([loc(1, 150)]);
  });

  it("drops forward entries when jumping while in the middle of history", () => {
    let history = createNavigationHistory();
    history = recordNavigation(history, loc(1, 100));
    history = recordNavigation(history, loc(5, 500));

    // Currently at page 10. Go back to page 5.
    const back1 = historyGoBack(history, loc(10, 1000));
    history = back1!.nextHistory;
    expect(back1!.target).toEqual(loc(5, 500));
    expect(canGoForward(history)).toBe(true);
    expect(history.forward).toEqual([loc(10, 1000)]);

    // From page 5, user jumps to page 20: forward history (page 10) must be dropped!
    history = recordNavigation(history, loc(5, 600));
    expect(canGoForward(history)).toBe(false);
    expect(history.forward).toHaveLength(0);
    expect(history.back).toEqual([loc(1, 100), loc(5, 600)]);
  });

  it("does not record consecutive entries on the same page (重複的一頁)", () => {
    let history = createNavigationHistory();
    history = recordNavigation(history, loc(2, 100));
    expect(history.back).toHaveLength(1);

    // Recording another jump while still on page 2 updates the entry's position but does not add a second entry
    history = recordNavigation(history, loc(2, 250));
    expect(history.back).toHaveLength(1);
    expect(history.back[0]).toEqual(loc(2, 250));

    // Jump from another page records normally
    history = recordNavigation(history, loc(4, 400));
    expect(history.back).toHaveLength(2);
    expect(history.back).toEqual([loc(2, 250), loc(4, 400)]);
  });

  it("enforces the 100 entries limit and drops the oldest", () => {
    let history = createNavigationHistory();
    for (let i = 1; i <= MAX_HISTORY_ENTRIES + 20; i++) {
      history = recordNavigation(history, loc(i, i * 10));
    }
    expect(history.back).toHaveLength(MAX_HISTORY_ENTRIES);
    // Oldest 20 entries (1 to 20) were dropped, so entries start from page 21
    expect(history.back[0]!.page).toBe(21);
    expect(history.back[MAX_HISTORY_ENTRIES - 1]!.page).toBe(MAX_HISTORY_ENTRIES + 20);
  });

  describe("adjusting history after page edits", () => {
    it("drops entries on deleted pages and shifts remaining entries", () => {
      let history = createNavigationHistory();
      history = recordNavigation(history, loc(1));
      history = recordNavigation(history, loc(3));
      history = recordNavigation(history, loc(5));

      // Delete 0-based page index 2 (page 3)
      history = adjustHistoryForEdit(
        history,
        { kind: "deletePages", pages: [2] },
        5,
      );

      // Page 1 stays page 1; Page 3 was deleted and dropped; Page 5 becomes page 4
      expect(history.back).toEqual([loc(1), loc(4)]);
    });

    it("shifts entries when pages are moved", () => {
      let history = createNavigationHistory();
      history = recordNavigation(history, loc(1));
      history = recordNavigation(history, loc(2));
      history = recordNavigation(history, loc(6));

      // Move 0-based page 1 (page 2) and page 5 (page 6) to before page 8 (end) of an 8-page document
      history = adjustHistoryForEdit(
        history,
        { kind: "movePages", pages: [1, 5], before: 8 },
        8,
      );

      // Page 1 stays page 1 (0-based 0)
      // Page 2 (0-based 1) moved to index 6 -> page 7
      // Page 6 (0-based 5) moved to index 7 -> page 8
      expect(history.back).toEqual([loc(1), loc(7), loc(8)]);
    });

    it("shifts entries when a blank page is inserted", () => {
      let history = createNavigationHistory();
      history = recordNavigation(history, loc(1));
      history = recordNavigation(history, loc(3));

      // Insert blank page at 0-based index 1 (between page 1 and page 2)
      history = adjustHistoryForEdit(
        history,
        { kind: "insertBlankPage", at: 1, like: 0 },
        4,
      );

      // Page 1 stays page 1; Page 3 shifts to page 4
      expect(history.back).toEqual([loc(1), loc(4)]);
    });

    it("deduplicates consecutive entries if an edit causes them to share the same page", () => {
      let history = createNavigationHistory();
      // Suppose we had page 1, page 2
      history = recordNavigation(history, loc(1));
      history = recordNavigation(history, loc(2));

      // Move page 2 (0-based 1) to page 1 position (before 0)
      history = adjustHistoryForEdit(
        history,
        { kind: "movePages", pages: [1], before: 0 },
        5,
      );
      // Now old page 2 became page 1, old page 1 became page 2: [loc(2), loc(1)]
      expect(history.back).toEqual([loc(2), loc(1)]);
    });

    it("clamps history to page count for imprecise edits (e.g. merging other files)", () => {
      let history = createNavigationHistory();
      history = recordNavigation(history, loc(1));
      history = recordNavigation(history, loc(5));
      history = recordNavigation(history, loc(10));

      // Document is truncated / reverted to 4 pages
      history = clampHistoryToPageCount(history, 4);

      expect(history.back).toEqual([loc(1)]);
    });
  });
});
