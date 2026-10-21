// A copy of the document without active content or hidden data (B3-15):
// what to strip is asked for here, then the main process asks where the copy goes.
// What came of it is said in the dialog itself: how many items of each category were removed.

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
import type { DocumentId, SanitizeArgs, SanitizeOptions, SanitizeReport, SanitizeResult } from "@/ipc/generated/contract";

const t = strings.sanitize;

export type SanitizeForm = {
  actions: boolean;
  attachments: boolean;
  formScripts: boolean;
  xfa: boolean;
  richMedia: boolean;
  hiddenData: boolean;
  keepWebLinks: boolean;
  annotations: "keep" | "remove" | "flatten";
};

export const DEFAULT_SANITIZE_FORM: SanitizeForm = {
  actions: true,
  attachments: true,
  formScripts: true,
  xfa: true,
  richMedia: true,
  hiddenData: true,
  keepWebLinks: true,
  annotations: "keep",
};

export function formToOptions(form: SanitizeForm): SanitizeOptions {
  return {
    actions: form.actions,
    attachments: form.attachments,
    formScripts: form.formScripts,
    xfa: form.xfa,
    richMedia: form.richMedia,
    hiddenData: form.hiddenData,
    keepWebLinks: form.keepWebLinks,
    annotations: form.annotations,
  };
}

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

  const annotRadioName = useId();
  const keptId = useId();

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
    onSanitize({ doc, options: formToOptions(form) }).then(
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

            <fieldset className="grid gap-2 text-sm">
              <legend className="mb-1 font-medium">{t.categoriesTitle}</legend>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.actions}
                  onChange={(e) => setForm((now) => ({ ...now, actions: e.target.checked }))}
                />
                {t.actions}
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.keepWebLinks}
                  onChange={(e) => setForm((now) => ({ ...now, keepWebLinks: e.target.checked }))}
                />
                {t.keepWebLinks}
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.attachments}
                  onChange={(e) => setForm((now) => ({ ...now, attachments: e.target.checked }))}
                />
                {t.attachments}
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.formScripts}
                  onChange={(e) => setForm((now) => ({ ...now, formScripts: e.target.checked }))}
                />
                {t.formScripts}
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.xfa}
                  onChange={(e) => setForm((now) => ({ ...now, xfa: e.target.checked }))}
                />
                {t.xfa}
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.richMedia}
                  onChange={(e) => setForm((now) => ({ ...now, richMedia: e.target.checked }))}
                />
                {t.richMedia}
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.hiddenData}
                  onChange={(e) => setForm((now) => ({ ...now, hiddenData: e.target.checked }))}
                />
                {t.hiddenData}
              </label>
            </fieldset>

            <fieldset className="grid gap-2 text-sm">
              <legend className="mb-1 font-medium">{t.annotationsTitle}</legend>
              {(["keep", "remove", "flatten"] as const).map((mode) => (
                <label key={mode} className="flex items-center gap-2">
                  <input
                    type="radio"
                    name={annotRadioName}
                    disabled={running}
                    checked={form.annotations === mode}
                    onChange={() => setForm((now) => ({ ...now, annotations: mode }))}
                  />
                  {t.annotations[mode]}
                </label>
              ))}
            </fieldset>

            <section aria-labelledby={keptId} className="space-y-1 text-sm">
              <h3 id={keptId} className="font-medium">
                {t.keptTitle}
              </h3>
              <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
                {t.kept.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </section>

            {sensitive && <p className="text-sm text-muted-foreground">{t.sensitiveNote}</p>}

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
