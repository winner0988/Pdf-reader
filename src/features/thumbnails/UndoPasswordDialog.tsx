// Undo of a document opened with a password (#94, option B): the password is not kept (MVP-16),
// and undo opens the document again, so it is asked for again each time.

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

const t = strings.pages.undoPassword;

type UndoPasswordDialogProps = {
  open: boolean;
  /** The last password typed did not open the document. */
  wrong: boolean;
  /** Undoes with `password`. */
  onUndo: (password: string) => void;
  onCancel: () => void;
};

export function UndoPasswordDialog({ open, wrong, onUndo, onCancel }: UndoPasswordDialogProps) {
  const [password, setPassword] = useState("");
  const fieldId = useId();
  const errorId = useId();

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (password === "") return;
    onUndo(password);
    // As when opening (MVP-16): the field never keeps a password once it is sent.
    setPassword("");
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          setPassword("");
          onCancel();
        }
      }}
    >
      <DialogContent className="sm:max-w-sm">
        <form onSubmit={submit} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>
          <div className="grid gap-2 text-sm">
            <label htmlFor={fieldId} className="font-medium">
              {t.label}
            </label>
            <input
              id={fieldId}
              type="password"
              autoFocus
              autoComplete="off"
              spellCheck={false}
              aria-invalid={wrong}
              aria-describedby={wrong ? errorId : undefined}
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              className="h-8 rounded-md border bg-background px-2 aria-invalid:border-destructive"
            />
            {wrong && (
              <p id={errorId} role="alert" className="text-destructive">
                {t.wrong}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                setPassword("");
                onCancel();
              }}
            >
              {t.cancel}
            </Button>
            <Button type="submit" disabled={password === ""}>
              {t.confirm}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
