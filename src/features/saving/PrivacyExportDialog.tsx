// The privacy export (B2-03): before the main process asks where to write the copy, what it
// removes and what it does not, so nobody takes the copy for anonymous when it is not.

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
import { strings } from "@/i18n/zh-TW";

const t = strings.privacyExport;

type PrivacyExportDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Writes the copy: `false` if the user closed the system's save dialog. */
  onExport: () => Promise<boolean>;
  /** Told once the copy is written. */
  onFinished: (message: string) => void;
};

export function PrivacyExportDialog({ open, onOpenChange, onExport, onFinished }: PrivacyExportDialogProps) {
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);
  const removedId = useId();
  const keptId = useId();

  // Every time it opens, it starts over.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setFailed(false);
  }

  const start = () => {
    setRunning(true);
    setFailed(false);
    onExport().then(
      (exported) => {
        setRunning(false);
        // False: the user closed the system's dialog; this one stays for another try.
        if (exported) {
          onOpenChange(false);
          onFinished(t.done);
        }
      },
      () => {
        setRunning(false);
        setFailed(true);
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
        <section aria-labelledby={removedId} className="space-y-1 text-sm">
          <h3 id={removedId} className="font-medium">
            {t.removedTitle}
          </h3>
          <ul className="list-disc space-y-1 pl-5">
            {t.removed.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </section>
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
        <p className="text-sm text-muted-foreground">{t.signatures}</p>
        {running && (
          <p role="status" className="text-sm">
            {t.running}
          </p>
        )}
        {failed && (
          <p role="alert" className="text-sm text-destructive">
            {t.failed}
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
