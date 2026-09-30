// "移到…" (B2-05): moving the selected pages without dragging, so the keyboard alone can do it.

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

const t = strings.pages.move;

type MovePagesDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Pages in the document. */
  pageCount: number;
  /** Pages being moved. */
  moving: number;
  /** Moves them just before this page (0-based; the page count: to the end). */
  onMove: (before: number) => void;
};

export function MovePagesDialog({ open, onOpenChange, pageCount, moving, onMove }: MovePagesDialogProps) {
  const [page, setPage] = useState("1");
  const [after, setAfter] = useState(false);
  const [invalid, setInvalid] = useState(false);
  const pageId = useId();
  const errorId = useId();

  // Every time it opens, it starts over.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setPage("1");
      setAfter(false);
      setInvalid(false);
    }
  }

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const number = Number(page);
    if (!Number.isInteger(number) || number < 1 || number > pageCount) {
      setInvalid(true);
      return;
    }
    onOpenChange(false);
    onMove(after ? number : number - 1);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        {/* Its own check, with its own message: the browser's would stop the form silently. */}
        <form noValidate onSubmit={submit} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description(moving)}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-2 text-sm">
            <label htmlFor={pageId} className="font-medium">
              {t.page}
            </label>
            <input
              id={pageId}
              type="number"
              inputMode="numeric"
              min={1}
              max={pageCount}
              value={page}
              aria-invalid={invalid || undefined}
              aria-describedby={invalid ? errorId : undefined}
              onChange={(event) => {
                setPage(event.target.value);
                setInvalid(false);
              }}
              className="h-8 w-28 rounded-md border bg-background px-2 aria-invalid:border-destructive"
            />
            {invalid && (
              <p id={errorId} role="alert" className="text-destructive">
                {t.outOfRange(pageCount)}
              </p>
            )}
            <fieldset className="flex gap-4">
              <legend className="sr-only">{t.position}</legend>
              <label className="flex items-center gap-1.5">
                <input type="radio" name="position" checked={!after} onChange={() => setAfter(false)} />
                {t.before}
              </label>
              <label className="flex items-center gap-1.5">
                <input type="radio" name="position" checked={after} onChange={() => setAfter(true)} />
                {t.after}
              </label>
            </fieldset>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t.cancel}
            </Button>
            <Button type="submit">{t.confirm}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
