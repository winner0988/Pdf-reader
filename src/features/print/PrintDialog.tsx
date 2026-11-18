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
import type { PrintLayoutKind, PrintPageOrder } from "@/features/print/layout";
import { MAX_PRINT_PAGES, pagesToPrint, type PrintRange } from "@/features/print/range";
import { PrintCancelled, type PrintPage } from "@/features/print/render";
import { strings } from "@/i18n/zh-TW";

export type PrintJob = {
  pages: PrintPage[];
  layout: PrintLayoutKind;
  order: PrintPageOrder;
  border: boolean;
};

type PrintDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  pageCount: number;
  currentPage: number;
  /** Renders the chosen pages for printing; rejects with `PrintCancelled` once `signal` aborts. */
  prepare: (pages: number[], onProgress: (done: number) => void, signal: AbortSignal) => Promise<PrintPage[]>;
  /** The pages and layout are ready: print them. */
  onReady: (job: PrintJob) => void;
  /** The author allows only low-resolution printing (MVP-19): the resolution it prints at. */
  lowResolutionDpi?: number;
  /** When printing is forbidden/disabled (e.g. permission check). */
  disabled?: boolean;
};

type Problem = "invalid" | "tooMany" | "failed";

/**
 * Chooses which pages to print and how they are arranged on the sheets (MVP-17, B5-15).
 * The system's print dialog comes next, for the printer, copies and orientation;
 * the pages are rendered in between, so a range keeps that short.
 */
export function PrintDialog({
  open,
  onOpenChange,
  pageCount,
  currentPage,
  prepare,
  onReady,
  lowResolutionDpi,
  disabled = false,
}: PrintDialogProps) {
  const t = strings.print;
  const [range, setRange] = useState<PrintRange["kind"]>("all");
  const [text, setText] = useState("");
  const [layout, setLayout] = useState<PrintLayoutKind>("1");
  const [order, setOrder] = useState<PrintPageOrder>("horizontal");
  const [border, setBorder] = useState(false);
  const [problem, setProblem] = useState<Problem | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const running = useRef<AbortController | null>(null);
  const previousActiveElement = useRef<HTMLElement | null>(null);
  const pagesId = useId();
  const problemId = useId();
  const layoutId = useId();
  const orderId = useId();
  const borderId = useId();

  // Every time it opens, it starts over with defaults.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setRange("all");
      setText("");
      setLayout("1");
      setOrder("horizontal");
      setBorder(false);
      setProblem(null);
      setProgress(null);
    }
  }

  useEffect(() => {
    if (open) {
      const active = document.activeElement as HTMLElement | null;
      if (active && !active.closest('[role="dialog"]')) {
        previousActiveElement.current = active;
      }
    }
  }, [open]);

  useEffect(() => () => running.current?.abort(), []);

  const close = () => {
    running.current?.abort();
    running.current = null;
    setProgress(null);
    onOpenChange(false);
    if (previousActiveElement.current) {
      const el = previousActiveElement.current;
      setTimeout(() => {
        if (document.contains(el)) {
          el.focus();
        }
      }, 0);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (disabled || progress) return;
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
        onReady({ pages, layout, order, border });
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

  const isMultiPage = layout === "2" || layout === "4" || layout === "6" || layout === "9";

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : close())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.note}</DialogDescription>
          {lowResolutionDpi !== undefined && (
            <DialogDescription>{strings.permissions.lowResNote(lowResolutionDpi)}</DialogDescription>
          )}
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <fieldset disabled={disabled || progress !== null} className="space-y-2 text-sm">
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

          <fieldset disabled={disabled || progress !== null} className="space-y-3 text-sm">
            <div className="space-y-1">
              <label htmlFor={layoutId} className="font-medium">
                {t.layout}
              </label>
              <select
                id={layoutId}
                className="h-8 w-full rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-50"
                value={layout}
                onChange={(e) => setLayout(e.target.value as PrintLayoutKind)}
              >
                <option value="1">{t.layouts["1"]}</option>
                <option value="2">{t.layouts["2"]}</option>
                <option value="4">{t.layouts["4"]}</option>
                <option value="6">{t.layouts["6"]}</option>
                <option value="9">{t.layouts["9"]}</option>
                <option value="booklet">{t.layouts.booklet}</option>
              </select>
            </div>

            {isMultiPage && (
              <div className="space-y-2 rounded-md border border-border p-3">
                <div className="flex items-center gap-2">
                  <label htmlFor={orderId} className="whitespace-nowrap">
                    {t.order}
                  </label>
                  <select
                    id={orderId}
                    className="h-8 rounded-md border border-input bg-background px-2 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 disabled:opacity-50"
                    value={order}
                    onChange={(e) => setOrder(e.target.value as PrintPageOrder)}
                  >
                    <option value="horizontal">{t.orders.horizontal}</option>
                    <option value="vertical">{t.orders.vertical}</option>
                  </select>
                </div>
                <label htmlFor={borderId} className="flex items-center gap-2">
                  <input
                    id={borderId}
                    type="checkbox"
                    checked={border}
                    onChange={(e) => setBorder(e.target.checked)}
                  />
                  <span>{t.pageBorder}</span>
                </label>
              </div>
            )}

            {layout === "booklet" && (
              <p className="rounded-md bg-muted p-2.5 text-xs text-muted-foreground leading-relaxed">
                {t.bookletHint}
              </p>
            )}
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
            <Button type="submit" disabled={disabled || progress !== null}>
              {t.next}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
