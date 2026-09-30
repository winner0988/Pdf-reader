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
import type { ExportApi, ExportJob } from "@/features/export/api";
import { pagesInRange, type PrintRange } from "@/features/print/range";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type DocumentId, type ExportFormat } from "@/ipc/generated/contract";

type ExportDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  pageCount: number;
  currentPage: number;
  api: ExportApi;
  /** The export ended: what to tell the user (how many pages were written). */
  onFinished: (message: string) => void;
};

type Problem = "invalid" | "tooMany" | "failed";

const RESOLUTIONS = [72, 150, 300];

/**
 * Chooses what to export (B2-04): the text or page images, and which pages. The main process
 * then asks where, in the system's dialog, and writes the files; this dialog shows the progress.
 */
export function ExportDialog({ open, onOpenChange, doc, pageCount, currentPage, api, onFinished }: ExportDialogProps) {
  const t = strings.export;
  const [kind, setKind] = useState<"text" | "png">("text");
  const [dpi, setDpi] = useState(150);
  const [range, setRange] = useState<PrintRange["kind"]>("all");
  const [text, setText] = useState("");
  const [problem, setProblem] = useState<Problem | null>(null);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  /** From "匯出…" until the export ends (the system's dialog comes first). */
  const [active, setActive] = useState(false);
  const running = useRef<ExportJob | null>(null);
  const written = useRef(0);
  const formatId = useId();
  const rangeId = useId();
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
  useEffect(
    () => () => {
      if (running.current) void api.cancel(running.current.request);
    },
    [api],
  );

  const close = () => {
    if (running.current) void api.cancel(running.current.request);
    onOpenChange(false);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (active) return;
    const chosen: PrintRange = range === "pages" ? { kind: "pages", text } : { kind: range };
    const result = pagesInRange(chosen, pageCount, currentPage, LIMITS.maxExportPages);
    if ("error" in result) {
      setProblem(result.error);
      return;
    }
    setProblem(null);
    written.current = 0;
    const format: ExportFormat = kind === "text" ? { kind: "text" } : { kind: "png", dpi };
    const job = api.exportPages(doc, result.pages, format, (done, total) => {
      written.current = done;
      setProgress({ done, total });
    });
    running.current = job;
    setActive(true);
    job.done.then(
      (exported) => {
        running.current = null;
        setActive(false);
        setProgress(null);
        // False: the user closed the system's dialog; this one stays for another try.
        if (exported) {
          onOpenChange(false);
          onFinished(t.done(result.pages.length));
        }
      },
      (error: unknown) => {
        running.current = null;
        setActive(false);
        setProgress(null);
        if (errorCodeOf(error) === "cancelled") {
          onOpenChange(false);
          onFinished(t.stopped(kind === "text" ? 0 : written.current));
        } else {
          setProblem("failed");
        }
      },
    );
  };

  const message =
    problem === "invalid"
      ? strings.print.invalid(pageCount)
      : problem === "tooMany"
        ? t.tooMany(LIMITS.maxExportPages)
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
          <fieldset disabled={active} className="space-y-2 text-sm">
            <legend id={formatId} className="font-medium">
              {t.format}
            </legend>
            <label className="flex items-center gap-2">
              <input type="radio" name="format" checked={kind === "text"} onChange={() => setKind("text")} />
              {t.text}
            </label>
            <div className="flex flex-wrap items-center gap-2">
              <label className="flex items-center gap-2">
                <input type="radio" name="format" checked={kind === "png"} onChange={() => setKind("png")} />
                {t.png}
              </label>
              <select
                aria-label={t.resolution}
                className="h-8 rounded-md border border-input bg-background px-2 disabled:opacity-50"
                disabled={kind !== "png"}
                value={dpi}
                onChange={(event) => setDpi(Number(event.target.value))}
              >
                {RESOLUTIONS.map((value) => (
                  <option key={value} value={value}>
                    {t.dpi(value)}
                  </option>
                ))}
              </select>
            </div>
          </fieldset>
          <fieldset disabled={active} className="space-y-2 text-sm">
            <legend id={rangeId} className="font-medium">
              {t.range}
            </legend>
            <label className="flex items-center gap-2">
              <input type="radio" name="range" checked={range === "all"} onChange={() => setRange("all")} />
              {strings.print.all(pageCount)}
            </label>
            <label className="flex items-center gap-2">
              <input type="radio" name="range" checked={range === "current"} onChange={() => setRange("current")} />
              {strings.print.current(currentPage)}
            </label>
            <div className="flex items-center gap-2">
              <input
                type="radio"
                name="range"
                aria-labelledby={pagesId}
                checked={range === "pages"}
                onChange={() => setRange("pages")}
              />
              <span id={pagesId}>{strings.print.pages}</span>
              <input
                type="text"
                aria-labelledby={pagesId}
                aria-invalid={problem === "invalid"}
                aria-describedby={message ? problemId : undefined}
                placeholder={strings.print.pagesPlaceholder}
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
              {t.progress(progress.done, progress.total)}
            </p>
          )}
          <DialogFooter>
            {active ? (
              <Button type="button" variant="outline" onClick={close}>
                {t.stop}
              </Button>
            ) : (
              <>
                <Button type="button" variant="outline" onClick={close}>
                  {t.cancel}
                </Button>
                <Button type="submit">{t.start}</Button>
              </>
            )}
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
