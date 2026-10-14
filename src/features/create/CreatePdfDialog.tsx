// Making a PDF of files (B3-03): before the main process asks which files, what can be used, what
// is only an experiment (the Office kinds), and what the conversion does not do, so nobody takes
// the PDF of a Word file for what Word makes of it.

import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { CreateApi } from "@/features/create/api";
import { createFailure } from "@/features/create/messages";
import { strings } from "@/i18n/zh-TW";

const t = strings.create;

type CreatePdfDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  api: CreateApi;
  /** Told once the PDF is made (and open in a tab). */
  onFinished: (message: string) => void;
};

export function CreatePdfDialog({ open, onOpenChange, api, onFinished }: CreatePdfDialogProps) {
  const [running, setRunning] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const kindsId = useId();
  const experimentalId = useId();
  const privacyId = useId();

  // Every time it opens, it starts over.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setProblem(null);
  }

  const start = () => {
    setRunning(true);
    setProblem(null);
    api.createPdf().then(
      (created) => {
        setRunning(false);
        // Null: the user closed one of the system's dialogs; this one stays for another try.
        if (created) {
          onOpenChange(false);
          onFinished(t.done(created.pages));
        }
      },
      (failure: unknown) => {
        setRunning(false);
        setProblem(createFailure(failure));
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !running && onOpenChange(next)}>
      <DialogContent className="max-h-[85vh] overflow-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>
        <section aria-labelledby={kindsId} className="space-y-1 text-sm">
          <h3 id={kindsId} className="font-medium">
            {t.kindsTitle}
          </h3>
          <ul className="list-disc space-y-1 pl-5">
            {t.kinds.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
          <p className="text-muted-foreground">{t.limits}</p>
        </section>
        <section aria-labelledby={experimentalId} className="space-y-1 rounded-md bg-muted px-3 py-2 text-sm">
          <h3 id={experimentalId} className="font-medium">
            {t.experimentalTitle}
          </h3>
          <p>{t.experimental}</p>
        </section>
        <section aria-labelledby={privacyId} className="space-y-1 text-sm">
          <h3 id={privacyId} className="font-medium">
            {t.privacyTitle}
          </h3>
          <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
            {t.privacy.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </section>
        {running && (
          <p role="status" className="text-sm">
            {t.running}
          </p>
        )}
        {problem !== null && (
          <p role="alert" className="text-sm text-destructive">
            {problem}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={running} onClick={() => onOpenChange(false)}>
            {t.cancel}
          </Button>
          <Button disabled={running} onClick={start}>
            {t.start}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
