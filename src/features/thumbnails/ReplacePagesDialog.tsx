// "取代頁面" (B6-05, #590): replacing selected pages with pages from another file.

import { useId, useState, type FormEvent } from "react";

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

const t = strings.pages.replace;

export type ReplacePagesDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 0-based first page index of the selected range. */
  first: number;
  /** Number of selected pages to be replaced. */
  count: number;
  /** Total pages in the source file. */
  sourcePages: number;
  /** Total pages in the current document. */
  docPages: number;
  /** Called with 0-based from and to indices. */
  onReplace: (from: number, to: number) => void;
  onCancel: () => void;
};

function rangeStr(start: number, end: number): string {
  return start === end ? `${start}` : `${start}–${end}`;
}

export function ReplacePagesDialog({
  open,
  onOpenChange,
  first,
  count,
  sourcePages,
  docPages,
  onReplace,
  onCancel,
}: ReplacePagesDialogProps) {
  const [from, setFrom] = useState("1");
  const [to, setTo] = useState(String(sourcePages));
  const fromId = useId();
  const toId = useId();
  const errorId = useId();

  // Reset inputs when opened
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setFrom("1");
      setTo(String(sourcePages));
    }
  }

  const fromNum = Number(from);
  const toNum = Number(to);
  const isFromInt = Number.isInteger(fromNum);
  const isToInt = Number.isInteger(toNum);
  const isFromRange = isFromInt && fromNum >= 1 && fromNum <= sourcePages;
  const isToRange = isToInt && toNum >= 1 && toNum <= sourcePages;
  const isOrderValid = fromNum <= toNum;

  let errorMessage: string | null = null;
  if (!isFromRange || !isToRange) {
    errorMessage = t.outOfRange(sourcePages);
  } else if (!isOrderValid) {
    errorMessage = t.fromGreaterThanTo;
  }

  const canSubmit = errorMessage === null;

  const firstPage = first + 1;
  const lastPage = first + count;

  // Preview calculation
  const newPages = isOrderValid && isFromRange && isToRange ? toNum - fromNum + 1 : 0;
  const totalAfter = docPages - count + newPages;

  const previewSegments: string[] = [];
  if (canSubmit) {
    if (first > 0) {
      const origRange = rangeStr(1, first);
      previewSegments.push(t.previewOriginal(origRange, origRange));
    }
    const srcNewRange = rangeStr(first + 1, first + newPages);
    const srcOrigRange = rangeStr(fromNum, toNum);
    previewSegments.push(t.previewSource(srcNewRange, srcOrigRange));

    const remainingAfter = docPages - (first + count);
    if (remainingAfter > 0) {
      const afterNewRange = rangeStr(first + newPages + 1, totalAfter);
      const afterOrigRange = rangeStr(first + count + 1, docPages);
      previewSegments.push(t.previewOriginal(afterNewRange, afterOrigRange));
    }
  }

  const handleOpenChange = (nextOpen: boolean) => {
    if (!nextOpen) {
      onCancel();
    }
    onOpenChange(nextOpen);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!canSubmit) return;
    onReplace(fromNum - 1, toNum - 1);
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form noValidate onSubmit={submit} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>
              {t.prompt(firstPage, lastPage, sourcePages)}
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-3 text-sm">
            <div className="flex flex-wrap items-center gap-2">
              <label htmlFor={fromId} className="font-medium">
                {t.from}
              </label>
              <input
                id={fromId}
                type="number"
                inputMode="numeric"
                min={1}
                max={sourcePages}
                value={from}
                aria-invalid={errorMessage !== null || undefined}
                aria-describedby={errorMessage !== null ? errorId : undefined}
                onChange={(e) => setFrom(e.target.value)}
                className="h-8 w-20 rounded-md border bg-background px-2 aria-invalid:border-destructive"
              />
              <label htmlFor={toId} className="font-medium">
                {t.to}
              </label>
              <input
                id={toId}
                type="number"
                inputMode="numeric"
                min={1}
                max={sourcePages}
                value={to}
                aria-invalid={errorMessage !== null || undefined}
                aria-describedby={errorMessage !== null ? errorId : undefined}
                onChange={(e) => setTo(e.target.value)}
                className="h-8 w-20 rounded-md border bg-background px-2 aria-invalid:border-destructive"
              />
              <span>
                {t.pageUnit} {t.sourceCount(sourcePages)} {t.targetLabel(firstPage, lastPage)}
              </span>
            </div>

            {errorMessage !== null && (
              <p id={errorId} role="alert" className="text-destructive text-sm">
                {errorMessage}
              </p>
            )}

            {canSubmit && (
              <div className="rounded-md border bg-muted/40 p-3 space-y-1.5 text-xs text-muted-foreground">
                <p className="font-medium text-foreground">{t.previewTitle(totalAfter)}</p>
                <ul className="list-disc pl-4 space-y-0.5">
                  {previewSegments.map((segment, idx) => (
                    <li key={idx}>{segment}</li>
                  ))}
                </ul>
              </div>
            )}
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
              {t.cancel}
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {t.confirm}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
