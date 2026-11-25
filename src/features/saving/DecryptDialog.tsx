// Saving a copy of an encrypted document without its password or restrictions (docs/architecture/decrypt-copy.md).
// The password goes to the main process and worker once and is not kept; nothing is saved in this window,
// and the field is emptied as soon as the dialog closes or the request fails or succeeds.

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
import { strings } from "@/i18n";
import { ipcErrorCode } from "@/ipc/errors";
import type { DecryptArgs, DocumentId } from "@/ipc/generated/contract";

const t = strings.decryption;

type DecryptDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  /** Writes the copy: `false` if the user closed the system's save dialog. */
  onDecrypt: (args: DecryptArgs) => Promise<boolean>;
  /** Told once the copy is written. */
  onFinished: (message: string) => void;
};

export function DecryptDialog({ open, onOpenChange, doc, onDecrypt, onFinished }: DecryptDialogProps) {
  const [password, setPassword] = useState("");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<"encrypted" | "notAllowed" | "signed" | "failed" | null>(null);
  const fieldId = useId();
  const errorId = useId();

  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    setPassword("");
    setError(null);
  }

  const start = (event: FormEvent) => {
    event.preventDefault();
    if (password === "" || running) return;
    setRunning(true);
    setError(null);
    const submittedPassword = password;
    setPassword("");
    onDecrypt({ doc, password: submittedPassword }).then(
      (written) => {
        setRunning(false);
        if (written) {
          onOpenChange(false);
          onFinished(t.done);
        }
      },
      (err) => {
        setRunning(false);
        const code = ipcErrorCode(err);
        if (code === "encrypted") {
          setError("encrypted");
        } else if (code === "notAllowed") {
          setError("notAllowed");
        } else if (code === "signed") {
          setError("signed");
        } else {
          setError("failed");
        }
      },
    );
  };

  const errorMessage =
    error === "encrypted"
      ? t.wrongPassword
      : error === "notAllowed"
        ? t.notAllowed
        : error === "signed"
          ? t.signed
          : error === "failed"
            ? t.failed
            : null;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!running) {
          if (!next) {
            setPassword("");
            setError(null);
          }
          onOpenChange(next);
        }
      }}
    >
      <DialogContent className="sm:max-w-md">
        <form onSubmit={start} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-2 text-sm">
            <label htmlFor={fieldId} className="font-medium">
              {t.password}
            </label>
            <input
              id={fieldId}
              type="password"
              autoFocus
              autoComplete="off"
              spellCheck={false}
              disabled={running}
              aria-invalid={errorMessage !== null}
              aria-describedby={errorMessage !== null ? errorId : undefined}
              value={password}
              onChange={(event) => {
                setError(null);
                setPassword(event.target.value);
              }}
              className="h-8 rounded-md border bg-background px-2 aria-invalid:border-destructive"
            />
            {errorMessage !== null && (
              <p id={errorId} role="alert" className="text-sm text-destructive">
                {errorMessage}
              </p>
            )}
          </div>
          <p role="status" className="min-h-5 text-sm text-muted-foreground">
            {running ? t.running : ""}
          </p>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={running}
              onClick={() => {
                setPassword("");
                setError(null);
                onOpenChange(false);
              }}
            >
              {t.cancel}
            </Button>
            <Button type="submit" disabled={password === "" || running}>
              {t.start}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
