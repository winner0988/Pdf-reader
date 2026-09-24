// Full-text search of the open document (MVP-10, docs/architecture/search.md). The main
// process searches page by page and streams the hits over a channel; this hook starts and
// cancels searches and keeps the results of the latest one only.

import { Channel, invoke } from "@tauri-apps/api/core";
import { useEffect, useReducer, useRef, useState, type ActionDispatch } from "react";

import {
  initialSearch,
  isSearched,
  reduceSearch,
  type SearchAction,
  type SearchQuery,
  type SearchState,
} from "@/features/search/model";
import { errorCodeOf } from "@/features/viewer/renderer";
import type { DocumentId, RequestId, SearchArgs, SearchEvent } from "@/ipc/generated/contract";
import { nextRequestId } from "@/ipc/requests";

export type SearchApi = {
  /** Resolves once the search has finished; rejects if it was cancelled or failed. */
  search(args: SearchArgs, onEvent: (event: SearchEvent) => void): Promise<void>;
  cancel(request: RequestId): Promise<void>;
};

export const tauriSearchApi: SearchApi = {
  search: (args, onEvent) => {
    const channel = new Channel<SearchEvent>();
    channel.onmessage = onEvent;
    return invoke<void>("search", { args, onEvent: channel });
  },
  cancel: (request) => invoke<void>("cancel", { request }),
};

/** A search starts once typing has paused this long (docs/ux/screen-map.md, section 3). */
export const SEARCH_DELAY_MS = 250;

export type SearchController = {
  query: string;
  caseSensitive: boolean;
  state: SearchState;
  /** Editing the query cancels the running search; a new one starts when typing pauses. */
  setQuery(query: string): void;
  setCaseSensitive(caseSensitive: boolean): void;
  /** An input method is composing text: its intermediate text is not searched. */
  setComposing(composing: boolean): void;
  /** Enter / F3: searches now if the query has not been searched yet, else selects the next (1) or previous (-1) hit. */
  submit(direction: 1 | -1): void;
  /** Closing the search bar: cancels the search and forgets its results (the query stays). */
  clear(): void;
};

type Running = { current: RequestId | null };
type Dispatch = ActionDispatch<[action: SearchAction]>;

function stop(api: SearchApi | undefined, running: Running) {
  if (running.current === null) return;
  // Best effort: the results of a cancelled search are ignored anyway.
  api?.cancel(running.current).catch(() => {});
  running.current = null;
}

function launch(api: SearchApi, running: Running, dispatch: Dispatch, doc: DocumentId, searched: SearchQuery) {
  stop(api, running);
  const request = nextRequestId();
  running.current = request;
  dispatch({ type: "start", request, searched });
  api
    .search({ request, doc, ...searched }, (event) => dispatch({ type: "event", request, event }))
    .catch((error: unknown) => {
      const code = errorCodeOf(error);
      // Cancelled: replaced by a newer search. Unknown document: the document was closed.
      if (code !== "cancelled" && code !== "unknownDocument") dispatch({ type: "failed", request });
    })
    .finally(() => {
      if (running.current === request) running.current = null;
    });
}

type UseSearchOptions = {
  /** Without it (demo data, tests without IPC) nothing is searched. */
  api?: SearchApi;
  doc?: DocumentId;
  /** The search bar is open: only then does typing start a search. */
  active: boolean;
  /** Tests pass 0. */
  delayMs?: number;
};

export function useSearch({ api, doc, active, delayMs = SEARCH_DELAY_MS }: UseSearchOptions): SearchController {
  const [query, setQueryText] = useState("");
  const [caseSensitive, setCaseSensitiveFlag] = useState(false);
  const [composing, setComposing] = useState(false);
  const [state, dispatch] = useReducer(reduceSearch, initialSearch);
  const running = useRef<RequestId | null>(null);

  // Another document: the results belonged to the old one (whose search ends when it closes).
  const [searchedDoc, setSearchedDoc] = useState(doc);
  if (searchedDoc !== doc) {
    setSearchedDoc(doc);
    dispatch({ type: "clear" });
  }

  const wanted: SearchQuery = { query, caseSensitive };
  const searchable = api !== undefined && doc !== undefined && query.trim() !== "";
  const due = active && searchable && !composing && !isSearched(state, wanted);

  useEffect(() => {
    if (!due || api === undefined || doc === undefined) return;
    const timer = window.setTimeout(() => launch(api, running, dispatch, doc, { query, caseSensitive }), delayMs);
    return () => window.clearTimeout(timer);
  }, [due, api, doc, query, caseSensitive, delayMs]);

  // A search never outlives its document or the window.
  useEffect(() => () => stop(api, running), [api, doc]);

  return {
    query,
    caseSensitive,
    state,
    setQuery(next) {
      if (next === query) return;
      stop(api, running);
      dispatch({ type: "clear" });
      setQueryText(next);
    },
    setCaseSensitive(next) {
      setCaseSensitiveFlag(next);
      if (searchable) launch(api, running, dispatch, doc, { query, caseSensitive: next });
    },
    setComposing,
    submit(direction) {
      if (!searchable) return;
      if (isSearched(state, wanted)) dispatch({ type: "step", direction });
      else launch(api, running, dispatch, doc, wanted);
    },
    clear() {
      stop(api, running);
      dispatch({ type: "clear" });
    },
  };
}
