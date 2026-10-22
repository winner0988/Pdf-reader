import { Ban, Search, ShieldAlert, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { groupMarksByPage, type RedactionMarkItem } from "@/features/redaction/model";
import { strings } from "@/i18n/zh-TW";
import type { AnnotationId, RedactionSummary } from "@/ipc/generated/contract";

const t = strings.redaction;

export type RedactionPanelProps = {
  marks: RedactionMarkItem[];
  summary?: RedactionSummary | null;
  allowed?: boolean;
  disabledReason?: string;
  signed?: boolean;
  onJumpToPage: (page: number) => void;
  onDeleteAll: () => void;
  onDeleteMark?: (page: number, markId: AnnotationId) => void;
  onApply: () => void;
  onOpenFindPersonalData?: () => void;
};

/** Redaction side panel: list marks, delete all, and apply (B2-21, ADR 0016). */
export function RedactionPanel({
  marks,
  summary = null,
  allowed = true,
  disabledReason,
  signed = false,
  onJumpToPage,
  onDeleteAll,
  onDeleteMark,
  onApply,
  onOpenFindPersonalData,
}: RedactionPanelProps) {
  const groups = groupMarksByPage(marks);
  const totalMarks = summary?.marks ?? marks.length;
  const totalPages = summary?.pages ?? groups.length;
  const canEdit = allowed && !signed;

  return (
    <div className="flex flex-col gap-3 p-1">
      <div>
        <h3 className="flex items-center gap-1.5 text-sm font-semibold">
          <ShieldAlert className="size-4 text-destructive" />
          {t.panelTitle}
        </h3>
        <p className="mt-1 text-xs text-muted-foreground">{t.panelDescription}</p>
      </div>

      {signed && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-300"
        >
          <Ban className="size-4 shrink-0" />
          <span>{t.signedDocument}</span>
        </div>
      )}

      {!allowed && !signed && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-300"
        >
          <Ban className="size-4 shrink-0" />
          <span>{disabledReason ?? t.notAllowed}</span>
        </div>
      )}

      {onOpenFindPersonalData && (
        <Button
          variant="outline"
          size="sm"
          className="w-full text-xs"
          disabled={!canEdit}
          onClick={onOpenFindPersonalData}
        >
          <Search className="mr-1 size-3.5" />
          {t.findPersonalData}
        </Button>
      )}

      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          className="flex-1 text-xs"
          disabled={!canEdit || marks.length === 0}
          onClick={onDeleteAll}
        >
          <Trash2 className="mr-1 size-3.5" />
          {t.deleteAll}
        </Button>
        <Button
          variant="destructive"
          size="sm"
          className="flex-1 text-xs"
          disabled={!canEdit || totalMarks === 0}
          onClick={onApply}
        >
          {t.apply}
        </Button>
      </div>

      {marks.length === 0 ? (
        <p className="py-6 text-center text-sm text-muted-foreground">{t.noMarks}</p>
      ) : (
        <>
          <div className="flex items-center justify-between text-xs text-muted-foreground">
            <span>{t.marksSummary(totalMarks, totalPages)}</span>
          </div>

          <div
            role="list"
            aria-label={t.marksListLabel}
            className="flex flex-col gap-1.5 overflow-auto pt-1"
          >
            {groups.map((group) => (
              <div
                key={group.page}
                role="listitem"
                className="flex items-center justify-between rounded-md border bg-card p-2 text-xs transition-colors hover:bg-accent/50"
              >
                <button
                  type="button"
                  className="font-medium hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary"
                  onClick={() => onJumpToPage(group.page + 1)}
                >
                  {t.pageMarks(group.page + 1, group.marks.length)}
                </button>
                {onDeleteMark && (
                  <div className="flex items-center gap-1">
                    {group.marks.map((mark, i) => (
                      <Button
                        key={mark.id}
                        variant="ghost"
                        size="icon-xs"
                        aria-label={`刪除第 ${group.page + 1} 頁標記 ${i + 1}`}
                        disabled={!canEdit}
                        onClick={() => onDeleteMark(group.page, mark.id)}
                      >
                        <Trash2 className="size-3 text-muted-foreground hover:text-destructive" />
                      </Button>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
