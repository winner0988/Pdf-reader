import { describe, expect, it } from "vitest";

import { initialSearch, isSearched, reduceSearch, type SearchAction, type SearchState } from "@/features/search/model";
import type { Quad, SearchEvent } from "@/ipc/generated/contract";

const quad = (x: number): Quad => ({
  ul: { x, y: 0 },
  ur: { x: x + 10, y: 0 },
  ll: { x, y: 10 },
  lr: { x: x + 10, y: 10 },
});

const hits = (pageIndex: number, ...xs: number[]): SearchEvent => ({
  kind: "hits",
  pageIndex,
  hits: xs.map((x) => ({ quads: [quad(x)] })),
});

const start = (request: number, query = "needle"): SearchAction => ({
  type: "start",
  request,
  searched: { query, caseSensitive: false },
});
const run = (...actions: SearchAction[]): SearchState => actions.reduce(reduceSearch, initialSearch);
const event = (request: number, e: SearchEvent): SearchAction => ({ type: "event", request, event: e });

describe("reduceSearch", () => {
  it("collects hits in document order and selects the first", () => {
    const state = run(
      start(7),
      event(7, hits(2, 10, 50)),
      event(7, { kind: "progress", pagesSearched: 5 }),
      event(7, hits(8, 30)),
      event(7, { kind: "done", totalHits: 3, truncated: false, noTextLayer: false }),
    );
    expect(state.hits.map((hit) => [hit.pageIndex, hit.quads[0]!.ul.x])).toEqual([
      [2, 10],
      [2, 50],
      [8, 30],
    ]);
    expect(state.current).toBe(0);
    expect(state.pagesSearched).toBe(5);
    expect(state.status).toBe("done");
  });

  it("ignores everything from an older search", () => {
    const state = run(
      start(1),
      event(1, hits(0, 1)),
      start(2),
      event(1, hits(3, 1)),
      event(1, { kind: "done", totalHits: 2, truncated: false, noTextLayer: false }),
      { type: "failed", request: 1 },
    );
    expect(state).toEqual({
      ...initialSearch,
      request: 2,
      searched: { query: "needle", caseSensitive: false },
      status: "searching",
    });
  });

  it("steps through hits and wraps around", () => {
    const found = run(start(1), event(1, hits(0, 1, 2, 3)));
    const step = (state: SearchState, direction: 1 | -1) => reduceSearch(state, { type: "step", direction });
    expect(step(found, 1).current).toBe(1);
    expect(step(step(step(found, 1), 1), 1).current).toBe(0);
    expect(step(found, -1).current).toBe(2);
    expect(step(initialSearch, 1)).toBe(initialSearch);
  });

  it("reports documents without text and cut-off results", () => {
    const none = run(start(1), event(1, { kind: "done", totalHits: 0, truncated: false, noTextLayer: true }));
    expect(none.noTextLayer).toBe(true);
    const many = run(start(1), event(1, { kind: "done", totalHits: 10000, truncated: true, noTextLayer: false }));
    expect(many.truncated).toBe(true);
  });

  it("a failed search is shown as failed; clearing forgets everything", () => {
    const failed = run(start(4), { type: "failed", request: 4 });
    expect(failed.status).toBe("failed");
    expect(reduceSearch(failed, { type: "clear" })).toEqual(initialSearch);
  });

  it("knows whether a query has been searched already", () => {
    const needle = { query: "needle", caseSensitive: false };
    const searching = run(start(1));
    expect(isSearched(searching, needle)).toBe(true);
    expect(isSearched(searching, { ...needle, caseSensitive: true })).toBe(false);
    expect(isSearched(searching, { ...needle, query: "needles" })).toBe(false);
    expect(isSearched(initialSearch, needle)).toBe(false);
    // A failed search is tried again.
    expect(isSearched(reduceSearch(searching, { type: "failed", request: 1 }), needle)).toBe(false);
  });
});
