import { useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { strings } from "@/i18n/zh-TW";
import type {
  AnnotationId,
  AnnotationSummaryItem,
  DocumentId,
} from "@/ipc/generated/contract";
import { type AnnotationsListApi, tauriAnnotationsListApi } from "./listApi";
import { type AnnotationFilterKind, matchesFilter } from "./listModel";
import { useAnnotationSummary } from "./useAnnotationSummary";

export type { AnnotationFilterKind };

export type AnnotationsListPanelProps = {
  doc?: DocumentId;
  active?: boolean;
  onJumpToAnnotation?: (pageIndex: number, id: AnnotationId) => void;
  api?: AnnotationsListApi;
  delayMs?: number;
};

const BATCH_SIZE = 50;

/**
 * Sidebar panel listing all annotations of the document, ordered by page and reading position (B4-06).
 */
export function AnnotationsListPanel({
  doc,
  active = true,
  onJumpToAnnotation,
  api = tauriAnnotationsListApi,
  delayMs = 300,
}: AnnotationsListPanelProps) {
  const state = useAnnotationSummary(doc, active, api, delayMs);
  const [filter, setFilter] = useState<AnnotationFilterKind>("all");
  const [focusedIndex, setFocusedIndex] = useState(0);
  const [renderedCount, setRenderedCount] = useState(BATCH_SIZE);
  const itemRefs = useRef<(HTMLDivElement | null)[]>([]);
  const containerRef = useRef<HTMLDivElement>(null);
  const filterSelectId = useId();

  const t = strings.annotationsList;
  const kinds = strings.annotations.kind;

  const items = useMemo(() => {
    if (state.status === "ready") return state.summary.items;
    if (state.status === "loading" && state.summary) return state.summary.items;
    return [];
  }, [state]);

  const truncated = state.status === "ready" ? state.summary.truncated : false;

  const filteredItems = useMemo(() => {
    return items.filter((item) => matchesFilter(item.kind, filter));
  }, [items, filter]);

  const visibleItems = useMemo(() => {
    return filteredItems.slice(0, renderedCount);
  }, [filteredItems, renderedCount]);

  const handleActivate = (item: AnnotationSummaryItem) => {
    onJumpToAnnotation?.(item.pageIndex, item.id);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (filteredItems.length === 0) return;

    let handled = true;
    switch (event.key) {
      case "ArrowDown": {
        const next = Math.min(focusedIndex + 1, filteredItems.length - 1);
        if (next >= renderedCount) {
          setRenderedCount((prev) => Math.min(prev + BATCH_SIZE, filteredItems.length));
        }
        setFocusedIndex(next);
        itemRefs.current[next]?.focus();
        break;
      }
      case "ArrowUp": {
        const prev = Math.max(focusedIndex - 1, 0);
        setFocusedIndex(prev);
        itemRefs.current[prev]?.focus();
        break;
      }
      case "Home": {
        setFocusedIndex(0);
        itemRefs.current[0]?.focus();
        break;
      }
      case "End": {
        const last = filteredItems.length - 1;
        setRenderedCount(filteredItems.length);
        setFocusedIndex(last);
        // Will focus in next render or microtask if expanding renderedCount
        setTimeout(() => itemRefs.current[last]?.focus(), 0);
        break;
      }
      case "Enter":
      case " ": {
        const current = filteredItems[focusedIndex];
        if (current) handleActivate(current);
        break;
      }
      default:
        handled = false;
    }

    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };

  const handleScroll = (event: React.UIEvent<HTMLDivElement>) => {
    const el = event.currentTarget;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 80) {
      setRenderedCount((prev) => Math.min(prev + BATCH_SIZE, filteredItems.length));
    }
  };

  if (state.status === "loading" && !state.summary) {
    return <p className="px-2 py-4 text-sm text-muted-foreground">{t.loading}</p>;
  }

  if (state.status === "failed") {
    return <p className="px-2 py-4 text-sm text-muted-foreground">{t.failed}</p>;
  }

  if (items.length === 0 && !truncated) {
    return <p className="px-2 py-4 text-sm text-muted-foreground">{t.empty}</p>;
  }

  return (
    <div
      ref={containerRef}
      data-testid="annotations-list-panel"
      className="flex flex-col gap-2 py-1 select-none"
      onCopy={(event) => {
        // Prevent copying annotations text to protect author's copy restriction
        event.preventDefault();
        event.stopPropagation();
      }}
      onScroll={handleScroll}
    >
      <div className="flex items-center justify-between gap-2 px-1">
        <label htmlFor={filterSelectId} className="text-xs text-muted-foreground shrink-0">
          {t.filterLabel}
        </label>
        <select
          id={filterSelectId}
          aria-label={t.filterLabel}
          data-testid="annotation-filter-select"
          value={filter}
          onChange={(e) => {
            setFilter(e.target.value as AnnotationFilterKind);
            setRenderedCount(BATCH_SIZE);
            setFocusedIndex(0);
          }}
          className="h-7 w-auto rounded border border-input bg-background px-2 text-xs text-foreground focus-visible:outline-2 focus-visible:outline-ring"
        >
          <option value="all">{t.filter.all}</option>
          <option value="textMark">{t.filter.textMark}</option>
          <option value="note">{t.filter.note}</option>
          <option value="textBox">{t.filter.textBox}</option>
          <option value="ink">{t.filter.ink}</option>
          <option value="stamp">{t.filter.stamp}</option>
          <option value="shape">{t.filter.shape}</option>
        </select>
      </div>

      {truncated && (
        <div
          role="note"
          data-testid="annotation-truncated-banner"
          className="rounded bg-muted px-2 py-1.5 text-xs text-muted-foreground"
        >
          {t.truncated}
        </div>
      )}

      {filteredItems.length === 0 ? (
        <p className="px-2 py-4 text-sm text-muted-foreground">{t.empty}</p>
      ) : (
        <div
          role="listbox"
          aria-label={t.title}
          tabIndex={0}
          onKeyDown={handleKeyDown}
          className="flex flex-col gap-1.5 outline-none"
        >
          {visibleItems.map((item, index) => {
            const kindName = kinds[item.kind];
            const pageText = t.page(item.pageIndex + 1);
            const hasText = item.text.trim().length > 0;
            const ariaLabel = t.itemLabel(kindName, item.pageIndex + 1, hasText ? item.text : undefined);
            const isFocused = focusedIndex === index;

            return (
              <div
                key={`${item.pageIndex}-${item.id}`}
                ref={(el) => {
                  itemRefs.current[index] = el;
                }}
                role="option"
                aria-selected={isFocused}
                aria-label={ariaLabel}
                tabIndex={isFocused ? 0 : -1}
                data-testid="annotation-list-item"
                data-annotation-kind={item.kind}
                data-page-index={item.pageIndex}
                onClick={() => {
                  setFocusedIndex(index);
                  handleActivate(item);
                }}
                className={`flex flex-col gap-1 rounded-lg border p-2 text-left cursor-pointer transition-colors outline-offset-1 focus-visible:outline-2 focus-visible:outline-primary ${
                  isFocused
                    ? "border-primary/50 bg-accent text-accent-foreground"
                    : "border-border bg-card text-card-foreground hover:bg-muted/50"
                }`}
              >
                <div className="flex items-center justify-between text-xs">
                  <span className="font-medium text-foreground">{kindName}</span>
                  <span className="text-muted-foreground">{pageText}</span>
                </div>
                {hasText && (
                  <p
                    className="truncate text-xs text-muted-foreground w-full"
                    title={item.text}
                  >
                    {item.text}
                  </p>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
