// Encrypting a copy of the document (B2-15, docs/architecture/encrypt-copy.md): the passwords and
// restrictions are asked for here, then the main process asks where the copy goes. The passwords go
// to the main process once and nothing keeps them (MVP-16): the fields are emptied as soon as the
// dialog is left or the copy is on its way.

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
import { argsOf, describeProblem, EMPTY_FORM, problemWith, type EncryptForm } from "@/features/saving/encrypt";
import { EncryptFields } from "@/features/saving/EncryptFields";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, EncryptArgs } from "@/ipc/generated/contract";

const t = strings.encryption;

type EncryptDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  /** Writes the copy: `false` if the user closed the system's save dialog. */
  onEncrypt: (args: EncryptArgs) => Promise<boolean>;
  /** Told once the copy is written. */
  onFinished: (message: string) => void;
};

export function EncryptDialog({ open, onOpenChange, doc, onEncrypt, onFinished }: EncryptDialogProps) {
  const [form, setForm] = useState<EncryptForm>(EMPTY_FORM);
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);

  // Every time it opens, it starts over, and nothing typed is kept when it closes.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    setForm(EMPTY_FORM);
    setFailed(false);
  }

  const problem = problemWith(form);
  const change = (changes: Partial<EncryptForm>) => {
    setFailed(false);
    setForm((now) => ({ ...now, ...changes }));
  };

  const start = (event: FormEvent) => {
    event.preventDefault();
    if (problem !== null || running) return;
    setRunning(true);
    setFailed(false);
    onEncrypt(argsOf(doc, form)).then(
      (written) => {
        setRunning(false);
        // False: the user closed the system's dialog; this one stays for another try.
        if (written) {
          setForm(EMPTY_FORM);
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
      <DialogContent className="max-h-[90vh] overflow-auto sm:max-w-lg">
        <form onSubmit={start} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>
          <EncryptFields form={form} onChange={change} disabled={running} />
          <p className="text-sm text-muted-foreground">{t.note}</p>
          <p className="text-sm text-muted-foreground">{t.signatures}</p>
          <p role="status" className="min-h-5 text-sm text-muted-foreground">
            {running ? t.running : problem !== null ? describeProblem(problem) : ""}
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
            <Button type="submit" disabled={problem !== null || running}>
              {t.start}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
