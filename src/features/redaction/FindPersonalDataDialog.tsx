import { useCallback, useState } from "react";
import { Loader2, Search, ShieldAlert, TriangleAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { AnnotationsApi } from "@/features/annotations/source";
import { strings } from "@/i18n/zh-TW";
import type {
  DocumentId,
  FindPersonalDataQuery,
  PersonalDataCategory,
  PersonalDataMatch,
  PersonalDataMatches,
} from "@/ipc/generated/contract";

const t = strings.redaction.findPersonalDataDialog;

export type FindPersonalDataDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc?: DocumentId;
  annotationsApi?: AnnotationsApi;
  onAddRedactions: (matches: PersonalDataMatch[]) => Promise<void>;
  onJumpToPage: (page: number) => void;
};

const CATEGORY_ITEMS: { id: PersonalDataCategory; label: string }[] = [
  { id: "taiwanId", label: t.categories.taiwanId },
  { id: "mobilePhone", label: t.categories.mobilePhone },
  { id: "landlinePhone", label: t.categories.landlinePhone },
  { id: "email", label: t.categories.email },
  { id: "creditCard", label: t.categories.creditCard },
  { id: "unifiedBusinessNumber", label: t.categories.unifiedBusinessNumber },
];

function categoryBadgeLabel(category: PersonalDataCategory): string {
  switch (category) {
    case "taiwanId":
      return "身分證／居留證";
    case "mobilePhone":
      return "手機";
    case "landlinePhone":
      return "市話";
    case "email":
      return "Email";
    case "creditCard":
      return "信用卡";
    case "unifiedBusinessNumber":
      return "統編";
    case "keyword":
      return "關鍵字";
  }
}

/** Dialog for scanning and finding personal data or custom keywords in memory (Issue #302). */
export function FindPersonalDataDialog({
  open,
  onOpenChange,
  doc,
  annotationsApi,
  onAddRedactions,
  onJumpToPage,
}: FindPersonalDataDialogProps) {
  const [selectedCats, setSelectedCats] = useState<Record<string, boolean>>({
    taiwanId: true,
    mobilePhone: true,
    landlinePhone: true,
    email: true,
    creditCard: true,
    unifiedBusinessNumber: true,
  });
  const [keywordEnabled, setKeywordEnabled] = useState(false);
  const [keyword, setKeyword] = useState("");
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<PersonalDataMatches | null>(null);
  const [selectedIndices, setSelectedIndices] = useState<Set<number>>(new Set());
  const [marking, setMarking] = useState(false);

  const toggleCategory = (cat: PersonalDataCategory) => {
    setSelectedCats((prev) => ({ ...prev, [cat]: !prev[cat] }));
  };

  const hasAnyCategory = Object.values(selectedCats).some(Boolean);
  const hasKeyword = keywordEnabled && keyword.trim().length > 0;
  const canSearch = !searching && !marking && doc !== undefined && (hasAnyCategory || hasKeyword);

  const handleSearch = useCallback(async () => {
    if (!annotationsApi?.findPersonalData || doc === undefined || (!hasAnyCategory && !hasKeyword)) {
      return;
    }
    setSearching(true);
    setError(null);
    try {
      const categories: PersonalDataCategory[] = [];
      for (const item of CATEGORY_ITEMS) {
        if (selectedCats[item.id]) {
          categories.push(item.id);
        }
      }
      const query: FindPersonalDataQuery = {
        categories,
        keyword: hasKeyword ? keyword.trim() : null,
      };
      const res = await annotationsApi.findPersonalData(doc, query);
      setResults(res);
      setSelectedIndices(new Set(res.matches.map((_, i) => i)));
    } catch {
      setError(t.failed);
      setResults(null);
    } finally {
      setSearching(false);
    }
  }, [annotationsApi, doc, hasAnyCategory, hasKeyword, keyword, selectedCats]);

  const toggleSelectAll = () => {
    if (!results) return;
    if (selectedIndices.size === results.matches.length) {
      setSelectedIndices(new Set());
    } else {
      setSelectedIndices(new Set(results.matches.map((_, i) => i)));
    }
  };

  const toggleMatch = (index: number) => {
    setSelectedIndices((prev) => {
      const next = new Set(prev);
      if (next.has(index)) {
        next.delete(index);
      } else {
        next.add(index);
      }
      return next;
    });
  };

  const handleMarkSelected = async () => {
    if (!results || selectedIndices.size === 0 || marking) return;
    setMarking(true);
    try {
      const selectedMatches = results.matches.filter((_, i) => selectedIndices.has(i));
      await onAddRedactions(selectedMatches);
      onOpenChange(false);
    } finally {
      setMarking(false);
    }
  };

  const close = () => {
    if (!searching && !marking) {
      onOpenChange(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (!next ? close() : onOpenChange(true))}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldAlert className="size-5 shrink-0 text-destructive" />
            {t.title}
          </DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-2 text-sm">
          {/* Category selection */}
          <div className="rounded-md border bg-muted/30 p-3 space-y-2.5">
            <p className="text-xs font-semibold text-foreground">{t.categoriesLabel}</p>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
              {CATEGORY_ITEMS.map((item) => (
                <label
                  key={item.id}
                  className="flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground cursor-pointer select-none"
                >
                  <input
                    type="checkbox"
                    checked={!!selectedCats[item.id]}
                    onChange={() => toggleCategory(item.id)}
                    disabled={searching || marking}
                    className="rounded border-input text-primary focus:ring-1 focus:ring-primary"
                  />
                  <span>{item.label}</span>
                </label>
              ))}
            </div>

            {/* Custom keyword */}
            <div className="pt-2 border-t flex flex-col sm:flex-row items-start sm:items-center gap-2">
              <label className="flex items-center gap-2 text-xs text-muted-foreground hover:text-foreground cursor-pointer select-none shrink-0">
                <input
                  type="checkbox"
                  checked={keywordEnabled}
                  onChange={(e) => setKeywordEnabled(e.target.value === "true" || e.target.checked)}
                  disabled={searching || marking}
                  className="rounded border-input text-primary focus:ring-1 focus:ring-primary"
                />
                <span>{t.customKeywordLabel}</span>
              </label>
              <input
                type="text"
                placeholder={t.customKeywordPlaceholder}
                value={keyword}
                disabled={!keywordEnabled || searching || marking}
                onChange={(e) => setKeyword(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && canSearch) {
                    e.preventDefault();
                    handleSearch();
                  }
                }}
                className="h-7 w-full sm:w-64 rounded-md border bg-background px-2 text-xs placeholder:text-muted-foreground/60 focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
              />
            </div>
          </div>

          {/* Search Trigger Button */}
          <div className="flex justify-end">
            <Button
              type="button"
              variant="default"
              size="sm"
              disabled={!canSearch}
              onClick={handleSearch}
              className="text-xs"
            >
              {searching ? (
                <>
                  <Loader2 className="mr-1.5 size-3.5 animate-spin" />
                  {t.searching}
                </>
              ) : (
                <>
                  <Search className="mr-1.5 size-3.5" />
                  {t.startSearch}
                </>
              )}
            </Button>
          </div>

          {/* Error display */}
          {error && (
            <p role="alert" className="text-xs text-destructive">
              {error}
            </p>
          )}

          {/* Results Area */}
          {results && (
            <div className="space-y-2 pt-2 border-t">
              <div className="flex items-center justify-between text-xs">
                <span className="font-medium text-foreground">
                  {t.matchesFound(results.matches.length)}
                </span>
                {results.matches.length > 0 && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={toggleSelectAll}
                    disabled={marking}
                    className="h-6 text-xs text-muted-foreground hover:text-foreground px-2"
                  >
                    {selectedIndices.size === results.matches.length ? t.deselectAll : t.selectAll}
                  </Button>
                )}
              </div>

              {results.hasMore && (
                <div
                  role="alert"
                  className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-300"
                >
                  <TriangleAlert className="size-4 shrink-0" />
                  <span>{t.hasMoreCaveat}</span>
                </div>
              )}

              {results.matches.length === 0 ? (
                <p className="py-6 text-center text-xs text-muted-foreground">{t.noMatches}</p>
              ) : (
                <div
                  role="list"
                  aria-label={t.title}
                  className="max-h-60 overflow-y-auto space-y-1 rounded-md border p-1.5 bg-card"
                >
                  {results.matches.map((match, idx) => {
                    const isChecked = selectedIndices.has(idx);
                    return (
                      <div
                        key={idx}
                        role="listitem"
                        className="flex items-center justify-between gap-2 rounded p-1.5 text-xs hover:bg-accent/50 transition-colors"
                      >
                        <label className="flex items-center gap-2 cursor-pointer select-none min-w-0 flex-1">
                          <input
                            type="checkbox"
                            checked={isChecked}
                            onChange={() => toggleMatch(idx)}
                            disabled={marking}
                            className="rounded border-input text-primary focus:ring-1 focus:ring-primary"
                          />
                          <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                            {categoryBadgeLabel(match.category)}
                          </span>
                          <span className="font-mono text-xs bg-muted/60 px-1.5 py-0.5 rounded text-foreground truncate select-all">
                            {match.preview}
                          </span>
                        </label>
                        <button
                          type="button"
                          onClick={() => onJumpToPage(match.page + 1)}
                          className="shrink-0 text-xs text-primary hover:underline px-1 py-0.5 rounded focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary"
                        >
                          {t.pageLabel(match.page + 1)}
                        </button>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}
        </div>

        <DialogFooter className="gap-2 sm:gap-0">
          <Button type="button" variant="outline" disabled={marking} onClick={close}>
            {results && results.matches.length > 0 ? t.close : t.cancel}
          </Button>
          {results && results.matches.length > 0 && (
            <Button
              type="button"
              variant="destructive"
              disabled={selectedIndices.size === 0 || marking}
              onClick={handleMarkSelected}
            >
              {marking ? (
                <>
                  <Loader2 className="mr-1.5 size-3.5 animate-spin" />
                  {t.markSelected(selectedIndices.size)}
                </>
              ) : (
                t.markSelected(selectedIndices.size)
              )}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
