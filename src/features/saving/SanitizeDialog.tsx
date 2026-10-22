// A copy of the document without active content or hidden data (B3-15):
// what to strip is asked for here, then the main process asks where the copy goes.
// What came of it is said in the dialog itself: how many items of each category were removed.

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
import { DEFAULT_SANITIZE_FORM, sanitizeOptionsOf, type SanitizeForm } from "@/features/saving/sanitize";
import { SanitizeFields } from "@/features/saving/SanitizeFields";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, SanitizeArgs, SanitizeReport, SanitizeResult } from "@/ipc/generated/contract";

const t = strings.sanitize;

type SanitizeDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  sensitive?: boolean;
  /** Writes the copy: `null` if the user closed the system's save dialog. */
  onSanitize: (args: SanitizeArgs) => Promise<SanitizeResult | null>;
};

export function SanitizeDialog({ open, onOpenChange, doc, sensitive, onSanitize }: SanitizeDialogProps) {
  const [form, setForm] = useState<SanitizeForm>(DEFAULT_SANITIZE_FORM);
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);
  const [result, setResult] = useState<SanitizeResult | null>(null);

  // Every time it opens, it starts over.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setForm(DEFAULT_SANITIZE_FORM);
      setFailed(false);
      setResult(null);
    }
  }

  const start = (event: FormEvent) => {
    event.preventDefault();
    if (running) return;
    setRunning(true);
    setFailed(false);
    onSanitize({ doc, options: sanitizeOptionsOf(form) }).then(
      (done) => {
        setRunning(false);
        // Null: the user closed the system's dialog; this one stays for another try.
        if (done) setResult(done);
      },
      () => {
        setRunning(false);
        setFailed(true);
      },
    );
  };

  const renderReportList = (report: SanitizeReport) => {
    const items: string[] = [];
    if (report.actions > 0) items.push(t.report.actions(report.actions));
    if (report.links > 0) items.push(t.report.links(report.links));
    if (report.attachments > 0) items.push(t.report.attachments(report.attachments));
    if (report.formScripts > 0) items.push(t.report.formScripts(report.formScripts));
    if (report.xfa > 0) items.push(t.report.xfa(report.xfa));
    if (report.richMedia > 0) items.push(t.report.richMedia(report.richMedia));
    if (report.hiddenData > 0) items.push(t.report.hiddenData(report.hiddenData));
    if (report.annotations > 0) items.push(t.report.annotations(report.annotations));

    if (items.length === 0) {
      return <p className="text-sm text-muted-foreground">{t.noItemsRemoved}</p>;
    }
    return (
      <ul className="list-disc space-y-1 pl-5 text-sm">
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    );
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !running && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-auto sm:max-w-lg">
        {result ? (
          <div className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.doneTitle}</DialogTitle>
              <DialogDescription>{t.doneDescription}</DialogDescription>
            </DialogHeader>
            <div role="status">{renderReportList(result.report)}</div>
            <DialogFooter>
              <Button type="button" onClick={() => onOpenChange(false)}>
                {t.close}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={start} className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.title}</DialogTitle>
              <DialogDescription>{t.description}</DialogDescription>
            </DialogHeader>

            <SanitizeFields
              form={form}
              onChange={(changes) => setForm((now) => ({ ...now, ...changes }))}
              disabled={running}
            />

            {sensitive && (
              <p className="text-sm text-muted-foreground">
                {form.hiddenData ? t.sensitiveNote : t.sensitiveKeptNote}
              </p>
            )}

            <p role="status" className="min-h-5 text-sm text-muted-foreground">
              {running ? t.running : ""}
            </p>
            {failed && (
              <p role="alert" className="text-sm text-destructive">
                {t.failed}
              </p>
            )}

            <DialogFooter>
              <Button type="button" variant="outline" disabled={running} onClick={() => onOpenChange(false)}>
                {t.cancel}
              </Button>
              <Button type="submit" disabled={running}>
                {t.start}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
