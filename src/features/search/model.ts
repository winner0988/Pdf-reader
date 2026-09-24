// Search results as the window shows them (MVP-10). Pure, so it is tested without IPC.

import { strings } from "@/i18n/zh-TW";
import { LIMITS, type Quad, type RequestId, type SearchEvent } from "@/ipc/generated/contract";

const t = strings.search;

export type SearchHitView = { pageIndex: number; quads: Quad[] };

/** What was searched for. */
export type SearchQuery = { query: string; caseSensitive: boolean };

export type SearchState = {
  /** The search whose results are shown; events of any other request are ignored. */
  request: RequestId | null;
  searched: SearchQuery | null;
  status: "idle" | "searching" | "done" | "failed";
  /** In document order (pages arrive in order). */
  hits: SearchHitView[];
  pagesSearched: number;
  truncated: boolean;
  noTextLayer: boolean;
  /** Index into `hits` of the selected hit, or -1. */
  current: number;
};

export type SearchAction =
  | { type: "start"; request: RequestId; searched: SearchQuery }
  | { type: "event"; request: RequestId; event: SearchEvent }
  | { type: "failed"; request: RequestId }
  | { type: "step"; direction: 1 | -1 }
  | { type: "clear" };

export const initialSearch: SearchState = {
  request: null,
  searched: null,
  status: "idle",
  hits: [],
  pagesSearched: 0,
  truncated: false,
  noTextLayer: false,
  current: -1,
};

export function reduceSearch(state: SearchState, action: SearchAction): SearchState {
  switch (action.type) {
    case "start":
      return { ...initialSearch, request: action.request, searched: action.searched, status: "searching" };
    case "clear":
      return initialSearch;
    case "step": {
      const count = state.hits.length;
      if (count === 0) return state;
      const from = state.current < 0 ? (action.direction === 1 ? -1 : 0) : state.current;
      return { ...state, current: (from + action.direction + count) % count };
    }
    case "failed":
      return action.request === state.request ? { ...state, status: "failed" } : state;
    case "event":
      break;
  }
  // Only the latest search counts: a cancelled one may still deliver what it already sent.
  if (action.request !== state.request) return state;
  const event = action.event;
  switch (event.kind) {
    case "hits": {
      const added = event.hits.map((hit) => ({ pageIndex: event.pageIndex, quads: hit.quads }));
      const hits = [...state.hits, ...added];
      return { ...state, hits, current: state.current < 0 ? 0 : state.current };
    }
    case "progress":
      return { ...state, pagesSearched: event.pagesSearched };
    case "done":
      return { ...state, status: "done", truncated: event.truncated, noTextLayer: event.noTextLayer };
  }
}

/** Whether `state` already shows (or is fetching) the results for `query`. */
export function isSearched(state: SearchState, query: SearchQuery): boolean {
  return (
    state.status !== "failed" &&
    state.searched?.query === query.query &&
    state.searched.caseSensitive === query.caseSensitive
  );
}

/** What the bar says in place of "第 n／N 筆" (docs/ux/screen-map.md, section 3). */
export function searchStatus(state: SearchState, pageCount: number): string {
  switch (state.status) {
    case "idle":
      return "";
    case "searching":
      return t.progress(state.pagesSearched, pageCount);
    case "failed":
      return t.failed;
    case "done":
      if (state.noTextLayer) return t.noTextLayer;
      if (state.hits.length === 0) return t.noResults(state.searched?.query.trim() ?? "");
      if (state.truncated) return t.truncated(LIMITS.maxSearchHits);
      return t.count(state.current + 1, state.hits.length);
  }
}
