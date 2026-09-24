import { ChevronDown, ChevronUp, X } from "lucide-react";
import { useEffect, useRef, type RefObject } from "react";

import { Button } from "@/components/ui/button";
import { searchStatus } from "@/features/search/model";
import type { SearchController } from "@/features/search/useSearch";
import { strings } from "@/i18n/zh-TW";
import { LIMITS } from "@/ipc/generated/contract";

const t = strings.search;

type SearchBarProps = {
  search: SearchController;
  pageCount: number;
  /** Lets Ctrl+F focus the field again while the bar is open. */
  inputRef?: RefObject<HTMLInputElement | null>;
  onClose: () => void;
};

export function SearchBar({ search, pageCount, inputRef, onClose }: SearchBarProps) {
  const ownRef = useRef<HTMLInputElement>(null);
  const input = inputRef ?? ownRef;
  const { state } = search;
  const status = searchStatus(state, pageCount);

  // Opening the bar selects what was searched before, so typing replaces it.
  useEffect(() => {
    input.current?.select();
  }, [input]);

  return (
    <div
      role="search"
      aria-label={t.label}
      className="absolute top-2 right-4 z-10 flex max-w-[calc(100%-2rem)] items-center gap-1 rounded-lg border bg-popover p-1.5 shadow-md"
    >
      <input
        ref={input}
        autoFocus
        aria-label={t.placeholder}
        placeholder={t.placeholder}
        className="h-8 w-56 min-w-24 shrink rounded-md border bg-background px-2 text-sm"
        value={search.query}
        onChange={(event) => {
          const next = event.target.value;
          if (new TextEncoder().encode(next).length <= LIMITS.maxQueryBytes) search.setQuery(next);
        }}
        onCompositionStart={() => search.setComposing(true)}
        onCompositionEnd={() => search.setComposing(false)}
        onKeyDown={(event) => {
          // Enter also confirms an input method's candidate: that is not a search.
          if (event.nativeEvent.isComposing) return;
          if (event.key === "Enter") {
            event.preventDefault();
            search.submit(event.shiftKey ? -1 : 1);
          } else if (event.key === "Escape") {
            onClose();
          }
        }}
      />
      <span data-status={state.status} className="px-1 text-xs whitespace-nowrap text-muted-foreground empty:hidden">
        {status}
      </span>
      {/* Progress changes ten times a second: only the outcome is announced. */}
      <span role="status" className="sr-only">
        {state.status === "searching" ? "" : status}
      </span>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={t.previous}
        disabled={state.hits.length === 0}
        onClick={() => search.submit(-1)}
      >
        <ChevronUp />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={t.next}
        disabled={state.hits.length === 0}
        onClick={() => search.submit(1)}
      >
        <ChevronDown />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        aria-label={t.caseSensitive}
        aria-pressed={search.caseSensitive}
        className="aria-pressed:bg-muted"
        onClick={() => search.setCaseSensitive(!search.caseSensitive)}
      >
        Aa
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t.close} onClick={onClose}>
        <X />
      </Button>
    </div>
  );
}
