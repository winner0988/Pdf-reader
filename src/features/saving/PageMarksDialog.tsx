// Dialog for adding page numbers, Bates numbers, and headers/footers to a PDF copy (B3-16).

import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  buildPageMarksPayload,
  calculatePreview,
  DEFAULT_PAGE_MARKS_FORM,
  findPositionConflicts,
  isBatesOverflow,
  pageMarksProblemWith,
  posLabel,
  type MarksPreview,
  type PageMarksForm,
} from "@/features/saving/pageMarks";
import { PageMarksFields } from "@/features/saving/PageMarksFields";
import { strings } from "@/i18n/zh-TW";
import type {
  DocumentId,
  PageMarkPosition,
  PageMarksCopyArgs,
} from "@/ipc/generated/contract";

const t = strings.pageMarks;

type PageMarksDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  totalPages: number;
  /** Writes the copy: `false` if the user closed the system's save dialog. */
  onPageMarksCopy: (args: PageMarksCopyArgs) => Promise<boolean>;
  onFinished: (message: string) => void;
};

export function PageMarksDialog({
  open,
  onOpenChange,
  doc,
  totalPages,
  onPageMarksCopy,
  onFinished,
}: PageMarksDialogProps) {
  const [form, setForm] = useState<PageMarksForm>(DEFAULT_PAGE_MARKS_FORM);
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);

  const preview = calculatePreview(form, totalPages);
  const conflicts = findPositionConflicts(form);
  const batesOverflow =
    form.enableBates &&
    isBatesOverflow(form.batesStart, form.batesDigits, preview.markedCount);
  const problem = pageMarksProblemWith(form);
  const noMarks = problem === t.noMarksError;

  const canSubmit =
    !running &&
    problem === null &&
    !batesOverflow &&
    preview.markedCount > 0;

  // Reset state on open
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setForm(DEFAULT_PAGE_MARKS_FORM);
      setFailed(false);
      setRunning(false);
    }
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;

    const payload = buildPageMarksPayload(form, totalPages);
    if (!payload) return;

    setRunning(true);
    setFailed(false);

    onPageMarksCopy({ doc, marks: payload }).then(
      (saved) => {
        setRunning(false);
        if (saved) {
          onOpenChange(false);
          onFinished(t.done);
        }
      },
      () => {
        setRunning(false);
        setFailed(true);
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !running && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <form onSubmit={onSubmit} className="grid gap-5">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>

          <PageMarksFields
            form={form}
            onChange={(changes) => setForm((prev) => ({ ...prev, ...changes }))}
            disabled={running}
            batesOverflow={Boolean(batesOverflow)}
            customPagesError={preview.markedCount === 0}
          />

          {/* Section 5: Interactive Preview */}
          <section className="rounded-md border p-3">
            <h3 className="font-medium text-sm">{t.previewTitle}</h3>
            {conflicts.length > 0 && (
              <p className="mt-1 text-xs text-destructive font-medium">
                {t.conflictError} {conflicts.map(posLabel).join("、")}
              </p>
            )}

            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              <PreviewPage label={t.firstPagePreview} page={preview.firstPage} conflicts={conflicts} />
              <PreviewPage label={t.lastPagePreview} page={preview.lastPage} conflicts={conflicts} />
            </div>
          </section>

          {/* Notes and Errors */}
          <div className="space-y-1 text-sm">
            <p className="text-muted-foreground">{t.signatures}</p>
            {noMarks && (
              <p className="text-xs text-muted-foreground">{t.noMarksError}</p>
            )}
            {running && <p className="text-xs text-primary">{t.running}</p>}
            {failed && <p className="text-xs text-destructive">{t.failed}</p>}
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={running}
              onClick={() => onOpenChange(false)}
            >
              {t.cancel}
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {t.start}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** One page of the preview: the marks in their six places, and in red a place two marks want. */
function PreviewPage({
  label,
  page,
  conflicts,
}: {
  label: string;
  page: MarksPreview["firstPage"];
  conflicts: PageMarkPosition[];
}) {
  const slot = (position: PageMarkPosition, align: string) => (
    <span
      className={`max-w-[32%] truncate ${align} ${
        conflicts.includes(position) ? "font-bold text-destructive" : ""
      }`}
    >
      {page?.slots[position] ?? " "}
    </span>
  );
  return (
    <div className="rounded border bg-muted/20 p-2.5">
      <div className="mb-1.5 text-xs font-medium text-muted-foreground">
        {label}
        {page && t.previewPageNumber(page.pageNumber)}
      </div>
      <div className="relative flex h-36 flex-col justify-between rounded border bg-background p-2 text-[11px] shadow-sm">
        <div className="flex justify-between gap-1 overflow-hidden">
          {slot("topLeft", "")}
          {slot("topCenter", "text-center")}
          {slot("topRight", "text-right")}
        </div>
        <div className="select-none text-center text-xs text-muted-foreground/30">
          {t.previewPageContent}
        </div>
        <div className="flex justify-between gap-1 overflow-hidden">
          {slot("bottomLeft", "")}
          {slot("bottomCenter", "text-center")}
          {slot("bottomRight", "text-right")}
        </div>
      </div>
    </div>
  );
}
