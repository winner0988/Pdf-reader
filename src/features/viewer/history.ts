// Navigation history for reader tabs (B4-01).
// Pure data structures and functions, tested without DOM or IPC.

import type { Zoom } from "@/features/shell/model";
import type { Edit } from "@/ipc/generated/contract";

export const MAX_HISTORY_ENTRIES = 100;

export type NavigationLocation = {
  page: number; // 1-based page number
  scrollTop: number;
  zoom: Zoom;
};

export type NavigationHistory = {
  back: NavigationLocation[];
  forward: NavigationLocation[];
};

export function createNavigationHistory(): NavigationHistory {
  return { back: [], forward: [] };
}

export function canGoBack(history: NavigationHistory): boolean {
  return history.back.length > 0;
}

export function canGoForward(history: NavigationHistory): boolean {
  return history.forward.length > 0;
}

/**
 * Deduplicates consecutive entries pointing to the same page.
 * "連續兩筆是同一頁時，不記第二筆。"
 */
export function deduplicateConsecutive(entries: readonly NavigationLocation[]): NavigationLocation[] {
  const result: NavigationLocation[] = [];
  for (const entry of entries) {
    if (result.length === 0 || result[result.length - 1]!.page !== entry.page) {
      result.push(entry);
    }
  }
  return result;
}

/**
 * Records the current location before a jump.
 * Consecutive entries on the same page are not recorded again.
 * Any forward entries are dropped (like browser navigation).
 */
export function recordNavigation(
  history: NavigationHistory,
  current: NavigationLocation,
): NavigationHistory {
  // If the last entry in back is already on the same page, do not add a duplicate entry.
  // Instead, update the scroll position and zoom of that existing entry to the latest position on that page.
  if (history.back.length > 0 && history.back[history.back.length - 1]!.page === current.page) {
    const updatedBack = [...history.back];
    updatedBack[updatedBack.length - 1] = current;
    return { back: updatedBack, forward: [] };
  }

  let nextBack = [...history.back, current];
  if (nextBack.length > MAX_HISTORY_ENTRIES) {
    nextBack = nextBack.slice(nextBack.length - MAX_HISTORY_ENTRIES);
  }

  return {
    back: nextBack,
    forward: [],
  };
}

/**
 * Navigates to the previous location in history.
 * Pushes the current location onto the forward stack.
 */
export function historyGoBack(
  history: NavigationHistory,
  current: NavigationLocation,
): { nextHistory: NavigationHistory; target: NavigationLocation } | null {
  if (history.back.length === 0) return null;

  const target = history.back[history.back.length - 1]!;
  const nextBack = history.back.slice(0, -1);

  let nextForward = history.forward;
  if (nextForward.length === 0 || nextForward[nextForward.length - 1]!.page !== current.page) {
    nextForward = [...nextForward, current];
    if (nextForward.length > MAX_HISTORY_ENTRIES) {
      nextForward = nextForward.slice(nextForward.length - MAX_HISTORY_ENTRIES);
    }
  }

  return {
    nextHistory: {
      back: nextBack,
      forward: nextForward,
    },
    target,
  };
}

/**
 * Navigates to the next location in history.
 * Pushes the current location onto the back stack.
 */
export function historyGoForward(
  history: NavigationHistory,
  current: NavigationLocation,
): { nextHistory: NavigationHistory; target: NavigationLocation } | null {
  if (history.forward.length === 0) return null;

  const target = history.forward[history.forward.length - 1]!;
  const nextForward = history.forward.slice(0, -1);

  let nextBack = history.back;
  if (nextBack.length === 0 || nextBack[nextBack.length - 1]!.page !== current.page) {
    nextBack = [...nextBack, current];
    if (nextBack.length > MAX_HISTORY_ENTRIES) {
      nextBack = nextBack.slice(nextBack.length - MAX_HISTORY_ENTRIES);
    }
  }

  return {
    nextHistory: {
      back: nextBack,
      forward: nextForward,
    },
    target,
  };
}

/**
 * Reconstructs the 0-based page order after `movePages` (`crates/pdf_worker/src/engine.rs`).
 * The unmoved pages stay in relative order; the moved pages are inserted at `start` in sorted order.
 */
export function pageOrderAfterMove(pageCount: number, moving: readonly number[], before: number): number[] {
  const moved = [...moving].sort((a, b) => a - b);
  const remaining: number[] = [];
  for (let i = 0; i < pageCount; i++) {
    if (!moved.includes(i)) remaining.push(i);
  }
  const start = before - moved.filter((p) => p < before).length;
  remaining.splice(start, 0, ...moved);
  return remaining;
}

/**
 * Maps a single 1-based page number to its new position after an edit,
 * or returns null if the page was deleted.
 */
export function mapPageAfterEdit(page: number, edit: Edit, pageCount: number): number | null {
  const oldIndex = page - 1;
  switch (edit.kind) {
    case "deletePages": {
      if (edit.pages.includes(oldIndex)) return null;
      const shift = edit.pages.filter((p) => p < oldIndex).length;
      return oldIndex - shift + 1;
    }
    case "movePages": {
      const order = pageOrderAfterMove(pageCount, edit.pages, edit.before);
      const newIndex = order.indexOf(oldIndex);
      return newIndex >= 0 ? newIndex + 1 : null;
    }
    case "insertBlankPage": {
      const newIndex = oldIndex >= edit.at ? oldIndex + 1 : oldIndex;
      return newIndex + 1;
    }
    default:
      return page;
  }
}

/**
 * Adjusts history entries when pages are edited (deleted, inserted, moved).
 * Pages that were deleted are dropped. Remaining entries follow their pages' new locations.
 * Consecutive entries that map to the same page are deduplicated.
 */
export function adjustHistoryForEdit(
  history: NavigationHistory,
  edit: Edit,
  pageCount: number,
): NavigationHistory {
  const adjustList = (list: readonly NavigationLocation[]): NavigationLocation[] => {
    const mapped: NavigationLocation[] = [];
    for (const entry of list) {
      const newPage = mapPageAfterEdit(entry.page, edit, pageCount);
      if (newPage !== null) {
        mapped.push({ ...entry, page: newPage });
      }
    }
    return deduplicateConsecutive(mapped);
  };

  return {
    back: adjustList(history.back),
    forward: adjustList(history.forward),
  };
}

/**
 * Drops entries beyond the document's new page count (for imprecise edits like merging external files,
 * or undo/redo where only the new page count is known).
 */
export function clampHistoryToPageCount(
  history: NavigationHistory,
  pageCount: number,
): NavigationHistory {
  const clampList = (list: readonly NavigationLocation[]): NavigationLocation[] => {
    const filtered = list.filter((entry) => entry.page >= 1 && entry.page <= pageCount);
    return deduplicateConsecutive(filtered);
  };

  return {
    back: clampList(history.back),
    forward: clampList(history.forward),
  };
}
