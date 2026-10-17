import { ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { strings } from "@/i18n/zh-TW";
import type { RedactionSummary } from "@/ipc/generated/contract";

const t = strings.redaction.dialog;

export type RedactionConfirmDialogProps = {
  open: boolean;
  summary: RedactionSummary | null;
  onConfirm: () => void;
  onCancel: () => void;
};

/** Confirmation dialog before applying security redactions (B2-21, ADR 0016). */
export function RedactionConfirmDialog({
  open,
  summary,
  onConfirm,
  onCancel,
}: RedactionConfirmDialogProps) {
  const reachingDeletions =
    summary && (summary.annotations > 0 || summary.fields > 0)
      ? t.reachingDeletions(summary.annotations, summary.fields)
      : t.reachingNone;

  return (
    <Dialog open={open} onOpenChange={(next) => (!next ? onCancel() : undefined)}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-destructive">
            <ShieldAlert className="size-5 shrink-0" />
            {t.title}
          </DialogTitle>
          <DialogDescription>
            {summary ? t.summary(summary.marks, summary.pages) : ""}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-3 py-2 text-sm">
          {/* Item 1: Reachable annotations and form fields deletion count */}
          <div
            className={`rounded-md p-2.5 text-xs ${
              summary && (summary.annotations > 0 || summary.fields > 0)
                ? "border border-amber-500/30 bg-amber-500/10 text-amber-900 dark:text-amber-200"
                : "border bg-muted/50 text-muted-foreground"
            }`}
          >
            <p>{reachingDeletions}</p>
          </div>

          {/* Item 2: Caveats of unhandled items */}
          <div className="space-y-1 rounded-md border bg-muted/40 p-2.5 text-xs text-muted-foreground">
            <p className="font-semibold text-foreground">{t.scopeCaveatsTitle}</p>
            <ul className="list-disc space-y-0.5 pl-4">
              {t.scopeCaveats.map((caveat, index) => (
                <li key={index}>{caveat}</li>
              ))}
            </ul>
          </div>

          {/* Item 3: Undo only until save */}
          <p className="text-xs text-muted-foreground">{t.undoCaveat}</p>

          {/* Item 4: Disk bytes not erased */}
          <p className="text-xs text-muted-foreground">{t.diskCaveat}</p>
        </div>

        <DialogFooter className="gap-2 sm:gap-0">
          <Button variant="outline" onClick={onCancel}>
            {t.cancel}
          </Button>
          <Button variant="destructive" onClick={onConfirm}>
            {t.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
