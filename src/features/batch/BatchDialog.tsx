// A batch (B3-04, docs/architecture/batch.md): what to do to many files, then the main process asks
// for the files and the folder in the system's dialogs and does them one after the other; this
// dialog shows how each went. The passwords of an encrypted copy go to the main process once and
// the fields are emptied when the batch starts, ends or the dialog closes.

import { useId, useRef, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { BatchApi, BatchJob } from "@/features/batch/api";
import { describeOutcome } from "@/features/batch/messages";
import { batchEncryptionOf, describeProblem, EMPTY_FORM, problemWith, type EncryptForm } from "@/features/saving/encrypt";
import { EncryptFields } from "@/features/saving/EncryptFields";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type BatchEvent, type BatchOperation, type BatchOutcome, type BatchReport } from "@/ipc/generated/contract";

const t = strings.batch;

type BatchDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  api: BatchApi;
};

type Kind = BatchOperation["kind"];

type Run = {
  files: string[];
  outcomes: (BatchOutcome | null)[];
  report: BatchReport | null;
  stopping: boolean;
};

export function BatchDialog({ open, onOpenChange, api }: BatchDialogProps) {
  const [kind, setKind] = useState<Kind>("removeMetadata");
  const [form, setForm] = useState<EncryptForm>(EMPTY_FORM);
  /** From "選擇檔案並開始…" until the batch ends (the system's dialogs come first). */
  const [active, setActive] = useState(false);
  const [run, setRun] = useState<Run | null>(null);
  const [failed, setFailed] = useState(false);
  const job = useRef<BatchJob | null>(null);
  const operationId = useId();

  // Every time it opens, it starts over, and nothing typed is kept when it closes.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    setForm(EMPTY_FORM);
    setRun(null);
    setFailed(false);
  }

  const problem = kind === "encrypt" ? problemWith(form) : null;
  const finished = run?.report != null;
  const doneCount = run ? run.outcomes.filter((outcome) => outcome !== null).length : 0;

  const onEvent = (event: BatchEvent) => {
    if (event.kind === "started") {
      setRun({ files: event.files, outcomes: event.files.map(() => null), report: null, stopping: false });
    } else {
      setRun((now) =>
        now
          ? { ...now, outcomes: now.outcomes.map((outcome, index) => (index === event.index ? event.outcome : outcome)) }
          : now,
      );
    }
  };

  const start = (event: FormEvent) => {
    event.preventDefault();
    if (problem !== null || active) return;
    const operation: BatchOperation = kind === "encrypt" ? batchEncryptionOf(form) : { kind: "removeMetadata" };
    setActive(true);
    setFailed(false);
    setRun(null);
    const started = api.run(operation, onEvent);
    job.current = started;
    started.done.then(
      (report) => {
        job.current = null;
        setActive(false);
        // Null: the user closed one of the system's dialogs; the dialog stays for another try.
        if (report === null) return;
        setForm(EMPTY_FORM);
        setRun((now) => (now ? { ...now, report } : { files: [], outcomes: [], report, stopping: false }));
      },
      () => {
        job.current = null;
        setActive(false);
        setRun(null);
        setFailed(true);
      },
    );
  };

  const stop = () => {
    const running = job.current;
    if (!running) return;
    setRun((now) => (now ? { ...now, stopping: true } : now));
    api.cancel(running.request).catch(() => {});
  };

  const status = (() => {
    if (finished && run?.report) return t.summary(run.report.done, run.report.failed, run.report.skipped);
    if (run?.stopping) return t.stopping;
    if (run && active) return t.running(doneCount, run.files.length);
    if (problem !== null) return describeProblem(problem);
    return "";
  })();

  return (
    <Dialog open={open} onOpenChange={(next) => !active && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-auto sm:max-w-lg">
        <form onSubmit={start} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>
          {run === null ? (
            <>
              <fieldset className="grid gap-2 text-sm" aria-describedby={operationId}>
                <legend className="mb-1 font-medium">{t.operation}</legend>
                {(["removeMetadata", "encrypt"] as const).map((choice) => (
                  <label key={choice} className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="batch-operation"
                      value={choice}
                      disabled={active}
                      checked={kind === choice}
                      onChange={() => setKind(choice)}
                    />
                    {t[choice]}
                  </label>
                ))}
              </fieldset>
              {kind === "encrypt" && (
                <EncryptFields form={form} onChange={(changes) => setForm((now) => ({ ...now, ...changes }))} disabled={active} />
              )}
              <ul id={operationId} className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">
                {t.notes(LIMITS.maxBatchFiles).map((note) => (
                  <li key={note}>{note}</li>
                ))}
              </ul>
            </>
          ) : (
            <ol aria-label={t.filesLabel} className="max-h-64 space-y-1 overflow-auto text-sm">
              {run.files.map((name, index) => (
                <li key={`${index}-${name}`} className="flex flex-wrap gap-x-2">
                  <span className="font-medium break-all">{name}</span>
                  <span className={run.outcomes[index]?.kind === "failed" ? "text-destructive" : "text-muted-foreground"}>
                    {describeOutcome(run.outcomes[index] ?? null)}
                  </span>
                </li>
              ))}
            </ol>
          )}
          <p role="status" className="min-h-5 text-sm">
            {status}
          </p>
          {failed && (
            <p role="alert" className="text-sm text-destructive">
              {t.failed}
            </p>
          )}
          <DialogFooter>
            {active ? (
              <Button type="button" variant="outline" disabled={!run || run.stopping} onClick={stop}>
                {t.stop}
              </Button>
            ) : finished ? (
              <Button type="button" onClick={() => onOpenChange(false)}>
                {t.close}
              </Button>
            ) : (
              <>
                <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                  {t.cancel}
                </Button>
                <Button type="submit" disabled={problem !== null}>
                  {t.start}
                </Button>
              </>
            )}
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
