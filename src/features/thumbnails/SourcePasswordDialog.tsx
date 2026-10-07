// The file whose pages are to go into the document (B2-06) is encrypted: its password is asked for.
// It goes to the document's worker for that one file and is wiped (MVP-16); nothing keeps it.

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

const t = strings.pages.sourcePassword;

type SourcePasswordDialogProps = {
  open: boolean;
  /** The last password typed did not open the file. */
  wrong: boolean;
  /** Tries `password` on the file. */
  onSubmit: (password: string) => void;
  onCancel: () => void;
};

export function SourcePasswordDialog({ open, wrong, onSubmit, onCancel }: SourcePasswordDialogProps) {
  const [password, setPassword] = useState("");
  const fieldId = useId();
  const errorId = useId();

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (password === "") return;
    onSubmit(password);
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
