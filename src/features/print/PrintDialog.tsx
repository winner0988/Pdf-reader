import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { MAX_PRINT_PAGES, pagesToPrint, type PrintRange } from "@/features/print/range";
import { PrintCancelled, type PrintPage } from "@/features/print/render";
import { strings } from "@/i18n/zh-TW";

type PrintDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pageCount: number;
  currentPage: number;
  /** Renders the chosen pages for printing; rejects with `PrintCancelled` once `signal` aborts. */
  prepare: (pages: number[], onProgress: (done: number) => void, signal: AbortSignal) => Promise<PrintPage[]>;
  /** The pages are ready: print them. */
  onReady: (pages: PrintPage[]) => void;
};

type Problem = "invalid" | "tooMany" | "failed";

/**
 * Chooses which pages to print (MVP-17). The system's print dialog comes next, for the printer,
 * copies and orientation; the pages are rendered in between, so a range keeps that short.
 */
export function PrintDialog({ open, onOpenChange, pageCount, currentPage, prepare, onReady }: PrintDialogProps) {
  const t = strings.print;
  const [range, setRange] = useState<PrintRange["kind"]>("all");
  const [text, setText] = useState("");
  const [problem, setProblem] = useState<Problem | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const running = useRef<AbortController | null>(null);
  const pagesId = useId();
  const problemId = useId();

  // Every time it opens, it starts over.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setRange("all");
      setText("");
      setProblem(null);
      setProgress(null);
    }
  }
  useEffect(() => () => running.current?.abort(), []);

  const close = () => {
    running.current?.abort();
    running.current = null;
    setProgress(null);
    onOpenChange(false);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (progress) return;
    const chosen: PrintRange = range === "pages" ? { kind: "pages", text } : { kind: range };
    const result = pagesToPrint(chosen, pageCount, currentPage);
    if ("error" in result) {
      setProblem(result.error);
      return;
    }
    setProblem(null);
    const controller = new AbortController();
    running.current = controller;
    setProgress({ done: 0, total: result.pages.length });
    prepare(result.pages, (done) => setProgress({ done, total: result.pages.length }), controller.signal).then(
      (pages) => {
        running.current = null;
        setProgress(null);
        onOpenChange(false);
        onReady(pages);
      },
      (error: unknown) => {
        if (error instanceof PrintCancelled || controller.signal.aborted) return;
        running.current = null;
        setProgress(null);
        setProblem("failed");
      },
    );
  };

  const message =
    problem === "invalid"
      ? t.invalid(pageCount)
      : problem === "tooMany"
        ? t.tooMany(MAX_PRINT_PAGES)
        : problem === "failed"
          ? t.failed
          : null;

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : close())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.note}</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <fieldset disabled={progress !== null} className="space-y-2 text-sm">
            <legend className="sr-only">{t.range}</legend>
            <label className="flex items-center gap-2">
              <input type="radio" name="range" checked={range === "all"} onChange={() => setRange("all")} />
              {t.all(pageCount)}
            </label>
            <label className="flex items-center gap-2">
              <input type="radio" name="range" checked={range === "current"} onChange={() => setRange("current")} />
              {t.current(currentPage)}
            </label>
            <div className="flex items-center gap-2">
              <input
                type="radio"
                name="range"
                aria-labelledby={pagesId}
                checked={range === "pages"}
                onChange={() => setRange("pages")}
              />
              <span id={pagesId}>{t.pages}</span>
              <input
                type="text"
                aria-labelledby={pagesId}
                aria-invalid={problem === "invalid"}
                aria-describedby={message ? problemId : undefined}
                placeholder={t.pagesPlaceholder}
                className="h-8 w-44 rounded-md border border-input bg-background px-2 outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20"
                value={text}
                onFocus={() => setRange("pages")}
                onChange={(event) => {
                  setRange("pages");
                  setText(event.target.value);
                  setProblem(null);
                }}
              />
            </div>
          </fieldset>
          {message && (
            <p id={problemId} role="alert" className="text-sm text-destructive">
              {message}
            </p>
          )}
          {progress && (
            <p role="status" className="text-sm text-muted-foreground">
              {t.preparing(progress.done, progress.total)}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={close}>
              {t.cancel}
            </Button>
            <Button type="submit" disabled={progress !== null}>
              {t.next}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
