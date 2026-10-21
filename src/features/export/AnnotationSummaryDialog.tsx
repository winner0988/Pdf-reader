// Annotation summary export dialog (Issue #306, B3-18).
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
import type { ExportApi } from "@/features/export/api";
import { formatAnnotationSummary } from "@/features/export/markdown";
import { strings } from "@/i18n/zh-TW";
import type { AnnotationSummaryFormat, DocumentId } from "@/ipc/generated/contract";

const t = strings.exportAnnotations;

export type AnnotationSummaryDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  docTitle: string;
  api: ExportApi;
  onFinished: (message: string) => void;
};

export function AnnotationSummaryDialog({
  open,
  onOpenChange,
  doc,
  docTitle,
  api,
  onFinished,
}: AnnotationSummaryDialogProps) {
  const [format, setFormat] = useState<AnnotationSummaryFormat>("text");
  const [includeTitle, setIncludeTitle] = useState(true);
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);

  // Reset state when opened
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setFormat("text");
      setIncludeTitle(true);
      setFailed(false);
      setRunning(false);
    }
  }

  const handleSubmit = async (e?: FormEvent) => {
    if (e) e.preventDefault();
    if (!api.getAnnotationSummary || !api.exportAnnotationSummary) return;
    setRunning(true);
    setFailed(false);
    try {
      const summary = await api.getAnnotationSummary(doc);
      const content = formatAnnotationSummary(summary, {
        format,
        includeTitle,
        docTitle,
      });
      const exported = await api.exportAnnotationSummary({
        doc,
        format,
        content,
      });
      setRunning(false);
      if (exported) {
        onOpenChange(false);
        onFinished(t.done);
      }
    } catch {
      setRunning(false);
      setFailed(true);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !running && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-auto sm:max-w-md">
        <form onSubmit={handleSubmit} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>

          <fieldset className="grid gap-2 text-sm">
            <legend className="mb-1 font-medium">{t.format}</legend>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="radio"
                name="annotation-summary-format"
                value="text"
                disabled={running}
                checked={format === "text"}
                onChange={() => setFormat("text")}
              />
              {t.text}
            </label>
            <label className="flex items-center gap-2 cursor-pointer">
              <input
                type="radio"
                name="annotation-summary-format"
                value="markdown"
                disabled={running}
                checked={format === "markdown"}
                onChange={() => setFormat("markdown")}
              />
              {t.markdown}
            </label>
          </fieldset>

          <div className="pt-1">
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                name="annotation-summary-include-title"
                disabled={running}
                checked={includeTitle}
                onChange={(e) => setIncludeTitle(e.target.checked)}
              />
              {t.includeTitle}
            </label>
          </div>

          {failed && (
            <p role="alert" className="text-sm text-destructive font-medium">
              {t.failed}
            </p>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={running}
            >
              {t.cancel}
            </Button>
            <Button type="submit" disabled={running}>
              {running ? t.running : t.start}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
