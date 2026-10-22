// The results of the latest search of the text index (B3-21): each document that has the words
// arrives on its own, the ones opened last first, then how many documents were searched.

import type { IndexEntryId, IndexSearchEvent, RequestId } from "@/ipc/generated/contract";

export type IndexResult = Extract<IndexSearchEvent, { kind: "found" }>;

export type IndexQuery = { query: string; caseSensitive: boolean };

export type IndexSearchState =
  | { kind: "idle" }
  | ({
      kind: "searching" | "done" | "failed";
      request: RequestId;
      results: IndexResult[];
      /** How many documents were searched: known once it is done. */
      documents: number | null;
    } & IndexQuery);

export type IndexSearchAction =
  | ({ type: "start"; request: RequestId } & IndexQuery)
  | { type: "event"; request: RequestId; event: IndexSearchEvent }
  | { type: "failed"; request: RequestId }
  /** The file of a result is gone or changed: it left the index, and leaves the list. */
  | { type: "gone"; entry: IndexEntryId }
  | { type: "clear" };

export const initialIndexSearch: IndexSearchState = { kind: "idle" };

export function reduceIndexSearch(state: IndexSearchState, action: IndexSearchAction): IndexSearchState {
  switch (action.type) {
    case "start":
      return {
        kind: "searching",
        request: action.request,
        query: action.query,
        caseSensitive: action.caseSensitive,
        results: [],
        documents: null,
      };
    case "clear":
      return initialIndexSearch;
    case "gone":
      if (state.kind === "idle") return state;
      return { ...state, results: state.results.filter((result) => result.entry !== action.entry) };
    default:
      // Events of a search that was replaced are ignored.
      if (state.kind !== "searching" || state.request !== action.request) return state;
      if (action.type === "failed") return { ...state, kind: "failed" };
      if (action.event.kind === "found") return { ...state, results: [...state.results, action.event] };
      return { ...state, kind: "done", documents: action.event.documents };
  }
}

/** Whether `wanted` is the query the state has results (or a running search) for. */
export function isSearched(state: IndexSearchState, wanted: IndexQuery): boolean {
  return (
    state.kind !== "idle" &&
    state.kind !== "failed" &&
    state.query === wanted.query &&
    state.caseSensitive === wanted.caseSensitive
  );
}
