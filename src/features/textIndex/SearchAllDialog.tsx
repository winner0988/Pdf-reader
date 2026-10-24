// "在所有文件中搜尋" (B3-21, ADR 0026, docs/ux/screen-map.md): the documents of the text index
// that have some words, the ones opened last first, each with its pages and a little text around
// the first hit on each. Choosing a page opens the document in a new tab at that page.

import { useEffect, useReducer, useRef, useState, type ActionDispatch } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { SEARCH_DELAY_MS } from "@/features/search/useSearch";
import { useSettings } from "@/features/settings/useSettings";
import type { TextIndexApi } from "@/features/textIndex/api";
import {
  initialIndexSearch,
  isSearched,
  reduceIndexSearch,
  type IndexQuery,
  type IndexResult,
  type IndexSearchAction,
  type IndexSearchState,
} from "@/features/textIndex/model";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type RequestId, type TabId } from "@/ipc/generated/contract";
import { nextRequestId } from "@/ipc/requests";

const t = strings.textIndex;

type Running = { current: RequestId | null };
type Dispatch = ActionDispatch<[action: IndexSearchAction]>;

function stop(api: TextIndexApi, running: Running) {
  if (running.current === null) return;
  // Best effort: the results of a cancelled search are ignored anyway.
  api.cancel(running.current).catch(() => {});
  running.current = null;
}

function launch(api: TextIndexApi, running: Running, dispatch: Dispatch, searched: IndexQuery) {
  stop(api, running);
  const request = nextRequestId();
  running.current = request;
  dispatch({ type: "start", request, ...searched });
  api
    .search({ request, ...searched }, (event) => dispatch({ type: "event", request, event }))
    .catch((error: unknown) => {
      // Cancelled: replaced by a newer search, or the dialog closed.
      if (errorCodeOf(error) !== "cancelled") dispatch({ type: "failed", request });
    })
    .finally(() => {
      if (running.current === request) running.current = null;
    });
}

/** What the line under the field says. */
function statusOf(state: IndexSearchState): string {
  switch (state.kind) {
    case "idle":
      return "";
    case "searching":
      return t.searching;
    case "failed":
      return t.failed;
    case "done":
      if (state.documents === 0) return t.empty;
      if (state.results.length === 0) return t.noResults(state.query);
      return t.found(state.results.length, state.documents ?? 0);
  }
}

type SearchAllDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  api: TextIndexApi;
  /** A result was opened in `tab`, to show `page` (1-based). */
  onOpened: (tab: TabId, page: number) => void;
  /** Tests pass 0. */
  delayMs?: number;
};

export function SearchAllDialog({ open, onOpenChange, api, onOpened, delayMs = SEARCH_DELAY_MS }: SearchAllDialogProps) {
  const settings = useSettings()?.settings;
  const on = settings !== undefined && settings.textIndex && settings.recordRecentFiles;
  const [query, setQueryText] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [composing, setComposing] = useState(false);
  const [state, dispatch] = useReducer(reduceIndexSearch, initialIndexSearch);
  const running = useRef<RequestId | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);

  // Every time it opens, the documents are searched anew: the index has changed meanwhile, and
  // a result is only valid until the next search.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    dispatch({ type: "clear" });
    setNotice(null);
  }

  const wanted: IndexQuery = { query, caseSensitive };
  const searchable = on && query.trim() !== "";
  const due = open && searchable && !composing && !isSearched(state, wanted);

  useEffect(() => {
    if (!due) return;
    const timer = window.setTimeout(() => launch(api, running, dispatch, { query, caseSensitive }), delayMs);
    return () => window.clearTimeout(timer);
  }, [due, api, query, caseSensitive, delayMs]);

  // A search never goes on with the dialog closed.
  useEffect(() => {
    if (!open) stop(api, running);
  }, [open, api]);
  useEffect(() => () => stop(api, running), [api]);

  const setQuery = (next: string) => {
    if (next === query) return;
    stop(api, running);
    dispatch({ type: "clear" });
    setQueryText(next);
  };

  const openResult = (result: IndexResult, pageIndex: number) => {
    if (opening) return;
    setOpening(true);
    setNotice(null);
    api
      .open(result.entry)
      .then(
        (tab) => onOpened(tab, pageIndex + 1),
        (error: unknown) => {
          if (errorCodeOf(error) === "unreadable") {
            dispatch({ type: "gone", entry: result.entry });
            setNotice(t.gone(result.displayName));
          } else {
            setNotice(t.openFailed);
          }
        },
      )
      .finally(() => setOpening(false));
  };

  const results = state.kind === "idle" ? [] : state.results;
  const status = statusOf(state);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[85vh] flex-col sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{on ? t.description : t.off}</DialogDescription>
        </DialogHeader>
        <div role="search" aria-label={t.title} className="flex items-center gap-2">
          <input
            autoFocus
            aria-label={t.label}
            placeholder={t.placeholder}
            disabled={!on}
            className="h-9 min-w-0 flex-1 rounded-md border bg-background px-2 text-sm disabled:opacity-50"
            value={query}
            onChange={(event) => {
              const next = event.target.value;
              if (new TextEncoder().encode(next).length <= LIMITS.maxQueryBytes) setQuery(next);
            }}
            onCompositionStart={() => setComposing(true)}
            onCompositionEnd={() => setComposing(false)}
            onKeyDown={(event) => {
              // Enter also confirms an input method's candidate: that is not a search.
              if (event.nativeEvent.isComposing || event.key !== "Enter") return;
              event.preventDefault();
              if (searchable && !isSearched(state, wanted)) launch(api, running, dispatch, wanted);
            }}
          />
          <Button
            variant="ghost"
            size="sm"
            aria-label={t.caseSensitive}
            aria-pressed={caseSensitive}
            disabled={!on}
            className="aria-pressed:bg-muted"
            onClick={() => {
              stop(api, running);
              dispatch({ type: "clear" });
              setCaseSensitive(!caseSensitive);
            }}
          >
            Aa
          </Button>
        </div>
        <p role="status" className="min-h-4 text-xs text-muted-foreground">
          {notice ?? status}
        </p>
        {results.length > 0 && (
          <ul aria-label={t.results} className="min-h-0 flex-1 space-y-3 overflow-auto text-sm">
            {results.map((result) => (
              <li key={result.entry} className="space-y-1">
                <div className="font-medium break-all">{result.displayName}</div>
                <div className="text-xs text-muted-foreground">
                  {t.hits(result.totalHits, result.pagesWithHits)}
                  {result.partial && ` · ${t.partial}`}
                </div>
                <ul className="space-y-0.5">
                  {result.pages.map((page) => (
                    <li key={page.pageIndex}>
                      <button
                        type="button"
                        disabled={opening}
                        className="w-full rounded px-2 py-1 text-left hover:bg-muted focus-visible:bg-muted disabled:opacity-50"
                        onClick={() => openResult(result, page.pageIndex)}
                      >
                        <span className="mr-2 text-xs whitespace-nowrap text-muted-foreground">
                          {t.page(page.pageIndex + 1)}
                        </span>
                        {page.snippet.before}
                        <mark className="rounded-sm bg-yellow-200 px-0.5 text-black">{page.snippet.matched}</mark>
                        {page.snippet.after}
                      </button>
                    </li>
                  ))}
                  {result.pagesWithHits > result.pages.length && (
                    <li className="px-2 text-xs text-muted-foreground">
                      {t.morePages(result.pagesWithHits - result.pages.length)}
                    </li>
                  )}
                </ul>
              </li>
            ))}
          </ul>
        )}
      </DialogContent>
    </Dialog>
  );
}
