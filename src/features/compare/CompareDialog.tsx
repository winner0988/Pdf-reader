// Text comparison dialog between two open tabs (B3-24, E9, docs/architecture/compare.md).
// Compares text between the current tab and another open tab.
// Diff computation and export generation occur directly in the main process.

import { useState } from "react";
import { AlertCircle, CheckCircle2, Info, Loader2, TriangleAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { CompareApi } from "@/features/compare/api";
import { strings } from "@/i18n/zh-TW";
import type {
  CompareExportFormat,
  CompareTextResult,
  DocumentId,
  TabId,
} from "@/ipc/generated/contract";

const t = strings.compare;

export type CompareTabOption = {
  tab: TabId;
  doc: DocumentId;
  displayName: string;
  copyAllowed: boolean;
};

export type CompareDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentDoc: DocumentId;
  currentDisplayName?: string;
  otherTabs: CompareTabOption[];
  api: CompareApi;
  onJumpToPage: (page: number) => void;
};

export function CompareDialog({
  open,
  onOpenChange,
  currentDoc,
  currentDisplayName,
  otherTabs,
  api,
  onJumpToPage,
}: CompareDialogProps) {
  const [selectedDoc, setSelectedDoc] = useState<DocumentId>(
    () => otherTabs.find((t) => t.copyAllowed)?.doc ?? otherTabs[0]?.doc ?? 0,
  );
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<CompareTextResult | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exportNotice, setExportNotice] = useState<string | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);

  const selectedTab = otherTabs.find((t) => t.doc === selectedDoc);
  const allOtherTabsDisabled = otherTabs.length > 0 && otherTabs.every((t) => !t.copyAllowed);
  const selectedDisabled = !selectedTab || !selectedTab.copyAllowed;

  const getErrorMessage = (err: unknown): string => {
    const code =
      typeof err === "object" && err !== null && "code" in err && typeof (err as { code: unknown }).code === "string"
        ? (err as { code: string }).code
        : undefined;
    if (code === "invalidArgument" || code === "notAllowed") {
      return t.copyNotAllowed;
    }
    if (code && code in strings.error.messages) {
      const msg = strings.error.messages[code as keyof typeof strings.error.messages];
      if (msg) return msg;
    }
    return t.failed;
  };

  const handleCompare = async () => {
    if (selectedDoc === undefined || selectedDisabled || allOtherTabsDisabled) return;
    setRunning(true);
    setError(null);
    setResult(null);
    setExportNotice(null);
    setExportError(null);
    try {
      const outcome = await api.compareText({ docA: currentDoc, docB: selectedDoc });
      setResult(outcome);
    } catch (err) {
      setError(getErrorMessage(err));
    } finally {
      setRunning(false);
    }
  };

  const handleExport = async (format: CompareExportFormat) => {
    if (selectedDoc === undefined) return;
    setExporting(true);
    setExportNotice(null);
    setExportError(null);
    try {
      const outcome = await api.exportCompareText({
        docA: currentDoc,
        docB: selectedDoc,
        format,
      });
      if (outcome.kind === "written") {
        setExportNotice(outcome.truncated ? t.exportTruncated(outcome.count) : t.exportDone(outcome.count));
      }
    } catch {
      setExportError(t.exportFailed);
    } finally {
      setExporting(false);
    }
  };

  const handleClose = () => {
    if (!running && !exporting) {
      onOpenChange(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent showCloseButton={false} className="sm:max-w-2xl max-h-[85vh] flex flex-col">
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4 flex-1 min-h-0 flex flex-col">
          {/* Target tab selection */}
          <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-2">
            <label htmlFor="compare-target-tab" className="text-sm font-medium shrink-0">
              {t.targetTab}
            </label>
            <select
              id="compare-target-tab"
              value={selectedDoc}
              onChange={(e) => {
                setSelectedDoc(Number(e.target.value));
                setResult(null);
                setError(null);
                setExportNotice(null);
                setExportError(null);
              }}
              disabled={running || exporting || allOtherTabsDisabled}
              aria-label={t.chooseTab}
              className="flex-1 h-9 rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary disabled:opacity-50"
            >
              {otherTabs.map((tab) => (
                <option key={tab.tab} value={tab.doc} disabled={!tab.copyAllowed}>
                  {tab.displayName}{!tab.copyAllowed ? `（${t.tabNoCopy}）` : ""}
                </option>
              ))}
            </select>
            <Button
              type="button"
              disabled={running || exporting || selectedDoc === undefined || selectedDisabled || allOtherTabsDisabled}
              onClick={handleCompare}
              className="shrink-0"
            >
              {running ? (
                <>
                  <Loader2 className="mr-1.5 size-4 animate-spin" />
                  {t.running}
                </>
              ) : (
                t.start
              )}
            </Button>
          </div>

          {/* Explanation if all tabs cannot copy */}
          {allOtherTabsDisabled && (
            <p role="alert" className="text-sm text-destructive">
              {t.allTabsNoCopy}
            </p>
          )}

          {/* Explanation if currently selected tab cannot copy */}
          {!allOtherTabsDisabled && selectedDisabled && (
            <p className="text-xs text-muted-foreground">
              {t.tabNoCopy}
            </p>
          )}

          {/* Error alert */}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}

          {/* Comparison results */}
          {result && (
            <div className="flex-1 min-h-0 flex flex-col gap-3 pt-2 border-t">
              {result.identical ? (
                <div className="flex items-center gap-2 rounded-md border border-emerald-500/30 bg-emerald-500/10 p-3 text-sm text-emerald-800 dark:text-emerald-300">
                  <CheckCircle2 className="size-4 shrink-0" />
                  <span>{t.identical}</span>
                </div>
              ) : result.empty ? (
                <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-300">
                  <AlertCircle className="size-4 shrink-0" />
                  <span>{t.noText}</span>
                </div>
              ) : (
                <>
                  {result.emptyA && (
                    <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-300">
                      <AlertCircle className="size-4 shrink-0" />
                      <span>{t.noTextOneSide(currentDisplayName || "目前分頁")}</span>
                    </div>
                  )}

                  {result.emptyB && (
                    <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-300">
                      <AlertCircle className="size-4 shrink-0" />
                      <span>{t.noTextOneSide(selectedTab?.displayName || "對照分頁")}</span>
                    </div>
                  )}

                  <div className="flex items-center justify-between text-sm font-medium text-foreground">
                    <span>{t.diffCount(result.items.length)}</span>
                  </div>

                  {result.truncated && (
                    <div
                      role="alert"
                      className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-300"
                    >
                      <TriangleAlert className="size-4 shrink-0" />
                      <span>{t.truncated}</span>
                    </div>
                  )}

                  {result.recognised && (
                    <div className="flex items-center gap-2 rounded-md border border-blue-500/30 bg-blue-500/10 p-2 text-xs text-blue-800 dark:text-blue-300">
                      <Info className="size-4 shrink-0" />
                      <span>{t.hasRecognised}</span>
                    </div>
                  )}

                  {result.pageTextTruncated && (
                    <div className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-300">
                      <TriangleAlert className="size-4 shrink-0" />
                      <span>{t.pageTextTruncated}</span>
                    </div>
                  )}

                  <div
                    role="region"
                    aria-label={t.diffList}
                    className="flex-1 min-h-0 overflow-y-auto max-h-[380px] rounded-md border p-2 bg-muted/20"
                  >
                    <div role="list" className="space-y-2">
                      {result.items.map((item, idx) => {
                        const ariaLabel = `${t.itemPages(item.pageA + 1, item.pageB + 1)}。${
                          item.deleted ? `${t.deletedLabel}：${item.deleted}。` : ""
                        }${item.inserted ? `${t.insertedLabel}：${item.inserted}。` : ""}`;

                        return (
                          <div key={idx} role="listitem">
                            <button
                              type="button"
                              onClick={() => onJumpToPage(item.pageA + 1)}
                              aria-label={ariaLabel}
                              className="w-full text-left p-2.5 rounded-md border border-border/60 bg-card hover:bg-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary transition-colors space-y-1.5 group"
                            >
                              <div className="flex items-center justify-between text-xs text-muted-foreground">
                                <span className="font-medium group-hover:text-primary transition-colors">
                                  {t.itemPages(item.pageA + 1, item.pageB + 1)}
                                </span>
                                {item.truncated && (
                                  <span className="italic text-[11px]">({t.itemTruncated})</span>
                                )}
                              </div>
                              {item.deleted && (
                                <div className="text-xs font-mono bg-destructive/10 text-destructive rounded px-2 py-1 break-words line-through whitespace-pre-wrap">
                                  <span className="sr-only">{t.deletedLabel}：</span>
                                  {item.deleted}
                                </div>
                              )}
                              {item.inserted && (
                                <div className="text-xs font-mono bg-emerald-500/10 text-emerald-700 dark:text-emerald-400 rounded px-2 py-1 break-words whitespace-pre-wrap">
                                  <span className="sr-only">{t.insertedLabel}：</span>
                                  {item.inserted}
                                </div>
                              )}
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  </div>

                  <div className="flex flex-wrap items-center gap-2 pt-1">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={exporting || running}
                      onClick={() => handleExport("text")}
                    >
                      {t.exportText}
                    </Button>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={exporting || running}
                      onClick={() => handleExport("markdown")}
                    >
                      {t.exportMarkdown}
                    </Button>
                    {exportNotice && (
                      <p role="status" className="text-xs text-muted-foreground">
                        {exportNotice}
                      </p>
                    )}
                    {exportError && (
                      <p role="alert" className="text-xs text-destructive">
                        {exportError}
                      </p>
                    )}
                  </div>
                </>
              )}
            </div>
          )}
        </div>

        <DialogFooter className="mt-2">
          <Button
            type="button"
            variant="outline"
            disabled={running || exporting}
            onClick={handleClose}
          >
            {t.close}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
