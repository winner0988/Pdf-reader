// The annotation summary (B3-18, docs/architecture/annotation-summary.md). The page only says what
// kind of file and whether it starts with the file name; the main process makes the file from what
// the worker found and asks where to save it. No text of the document passes through the page.
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
import type { AnnotationSummaryApi } from "@/features/export/api";
import { strings } from "@/i18n/zh-TW";
import type { AnnotationSummaryFormat, DocumentId } from "@/ipc/generated/contract";

const t = strings.exportAnnotations;

type AnnotationSummaryDialogProps = {
  doc: DocumentId;
  api: AnnotationSummaryApi;
  onClose: () => void;
  /** The file was written: what to tell the user (how many annotations it lists). */
  onFinished: (message: string) => void;
};

export function AnnotationSummaryDialog({ doc, api, onClose, onFinished }: AnnotationSummaryDialogProps) {
  const [format, setFormat] = useState<AnnotationSummaryFormat>("text");
  const [includeName, setIncludeName] = useState(true);
  const [state, setState] = useState<"idle" | "running" | "empty" | "failed">("idle");
  const running = state === "running";

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setState("running");
    try {
      const outcome = await api.exportAnnotationSummary({ doc, format, includeName });
      if (outcome.kind === "written") {
        onClose();
        onFinished(outcome.truncated ? t.truncated(outcome.count) : t.done(outcome.count));
      } else {
        // Nothing to list: say so here. The save dialog closed: choose again or cancel.
        setState(outcome.kind === "empty" ? "empty" : "idle");
      }
    } catch {
      setState("failed");
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && !running && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="space-y-4">
          <fieldset disabled={running} className="space-y-2 text-sm">
            <legend className="font-medium">{t.format}</legend>
            <label className="flex items-center gap-2">
              <input type="radio" name="format" checked={format === "text"} onChange={() => setFormat("text")} />
              {t.text}
            </label>
            <label className="flex items-center gap-2">
              <input
                type="radio"
                name="format"
                checked={format === "markdown"}
                onChange={() => setFormat("markdown")}
              />
              {t.markdown}
            </label>
          </fieldset>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              disabled={running}
              checked={includeName}
              onChange={(event) => setIncludeName(event.target.checked)}
            />
            {t.includeName}
          </label>
          {state === "empty" && (
            <p role="status" className="text-sm text-muted-foreground">
              {t.empty}
            </p>
          )}
          {state === "failed" && (
            <p role="alert" className="text-sm text-destructive">
              {t.failed}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onClose} disabled={running}>
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
