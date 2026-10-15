// Marking a document as sensitive, or taking the mark out (B3-05, ADR 0004, ADR 0010): what it
// does, and what will happen to the file, are said before anything is written; a signed, read-only
// or changed file, or one with changes not saved, is refused with the reason. An encrypted file
// needs its permissions password, which is never kept.

import { useEffect, useId, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { SensitiveApi } from "@/features/sensitive/api";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, ErrorCode, MarkCheck } from "@/ipc/generated/contract";

const t = strings.sensitive;

type SensitiveDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  /** The document is marked now: the dialog takes the mark out. */
  sensitive: boolean;
  api: Pick<SensitiveApi, "check" | "set">;
  /** Told once the file is written. */
  onFinished: (message: string) => void;
};

/** What the dialog says of the file: being checked, why it cannot be written, or that it can. */
type Checked = { kind: "checking" } | { kind: "failed" } | { kind: "checked"; check: MarkCheck };

/** Why the file cannot be written, said with the reason only: nothing to confirm. */
const REFUSALS: Partial<Record<MarkCheck, string>> = {
  signed: t.signed,
  readOnly: t.readOnly,
  unsaved: t.unsaved,
  changedOnDisk: t.changedOnDisk,
};

/** Why writing failed, from the main process's error: the file is as it was. */
function failure(code: ErrorCode): string {
  switch (code) {
    case "encrypted":
      return t.wrongPassword;
    case "notAllowed":
      return t.notOwnerPassword;
    case "signed":
      return t.signed;
    case "readOnly":
      return t.readOnly;
    case "changedOnDisk":
      return t.changedOnDisk;
    case "fileInUse":
      return t.fileInUse;
    case "corrupted":
      return t.repaired;
    default:
      return t.failed;
  }
}

export function SensitiveDialog({ open, onOpenChange, doc, sensitive, api, onFinished }: SensitiveDialogProps) {
  const [checked, setChecked] = useState<Checked>({ kind: "checking" });
  const [password, setPassword] = useState("");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fieldId = useId();
  const errorId = useId();

  // Every time it opens, it starts over, and the file is checked again: it may have changed.
  const [asked, setAsked] = useState<{ open: boolean; doc: DocumentId | null }>({ open: false, doc: null });
  if (open !== asked.open || (open && doc !== asked.doc)) {
    setAsked({ open, doc: open ? doc : null });
    setPassword("");
    setError(null);
    setRunning(false);
    setChecked({ kind: "checking" });
  }
  useEffect(() => {
    if (!open) return;
    let current = true;
    api.check(doc).then(
      (check) => current && setChecked({ kind: "checked", check }),
      () => current && setChecked({ kind: "failed" }),
    );
    return () => {
      current = false;
    };
  }, [open, doc, api]);

  const check = checked.kind === "checked" ? checked.check : null;
  const refusal = check === null ? null : (REFUSALS[check] ?? null);
  const needsPassword = check === "needsPassword";
  const writable = check === "ready" || needsPassword;

  const close = () => {
    // The field never keeps a password once the dialog is gone.
    setPassword("");
    onOpenChange(false);
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!writable || running || (needsPassword && password === "")) return;
    setRunning(true);
    setError(null);
    const typed = needsPassword ? password : null;
    setPassword("");
    api.set(doc, !sensitive, typed).then(
      () => {
        setRunning(false);
        onOpenChange(false);
        onFinished(sensitive ? t.unmarked : t.marked);
      },
      (reason: unknown) => {
        setRunning(false);
        setError(failure(errorCodeOf(reason)));
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : !running && close())}>
      <DialogContent className="max-h-[85vh] overflow-auto sm:max-w-lg">
        <form onSubmit={submit} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{sensitive ? t.unmarkTitle : t.markTitle}</DialogTitle>
            <DialogDescription>{sensitive ? t.whatUnmarkDoes : t.whatItDoes[0]}</DialogDescription>
          </DialogHeader>
          {!sensitive && (
            <ul className="list-disc space-y-1 pl-5 text-sm">
              {t.whatItDoes.slice(1).map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          )}
          <div className="grid gap-2 text-sm">
            {checked.kind === "checking" && <p role="status">{t.checking}</p>}
            {checked.kind === "failed" && (
              <p role="alert" className="text-destructive">
                {t.checkFailed}
              </p>
            )}
            {refusal && <p role="alert">{refusal}</p>}
            {writable && <p>{t.ready}</p>}
            {needsPassword && (
              <>
                <p>{t.needsPassword}</p>
                <label htmlFor={fieldId} className="font-medium">
                  {t.passwordLabel}
                </label>
                <input
                  id={fieldId}
                  type="password"
                  autoFocus
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={error !== null}
                  aria-describedby={error !== null ? errorId : undefined}
                  value={password}
                  disabled={running}
                  onChange={(event) => setPassword(event.target.value)}
                  className="h-8 rounded-md border bg-background px-2 aria-invalid:border-destructive"
                />
              </>
            )}
            {running && <p role="status">{t.running}</p>}
            {error && (
              <p id={errorId} role="alert" className="text-destructive">
                {error}
              </p>
            )}
          </div>
          <DialogFooter>
            {writable ? (
              <>
                <Button type="button" variant="outline" disabled={running} onClick={close}>
                  {t.cancel}
                </Button>
                <Button type="submit" disabled={running || (needsPassword && password === "")}>
                  {sensitive ? t.unmark : t.mark}
                </Button>
              </>
            ) : (
              <Button type="button" onClick={close}>
                {t.close}
              </Button>
            )}
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
