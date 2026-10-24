// Signing a document (B3-06, ADR 0020, docs/architecture/signing.md): whether it can be signed is
// said when the dialog opens; then the certificate file (chosen in the main process's dialog), its
// password, where the signature shows and why. The main process asks where the signed copy goes,
// verifies it, and opens it in a tab of its own; the dialog says so, or why it did not work.

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
import type { SigningApi } from "@/features/signing/api";
import {
  argsOf,
  CORNERS,
  DEFAULT_FORM,
  failure,
  reasonFits,
  REFUSALS,
  type SignForm,
} from "@/features/signing/signing";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, SignCheck, SigningFile, SignResult } from "@/ipc/generated/contract";

const t = strings.signing;

type SignDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  /** The page shown, 1-based: a visible signature goes on it. */
  page: number;
  api: SigningApi;
};

/** What the dialog knows of the document: being checked, or whether it can be signed. */
type Checked = { kind: "checking" } | { kind: "failed" } | { kind: "checked"; check: SignCheck };

export function SignDialog({ open, onOpenChange, doc, page, api }: SignDialogProps) {
  const [checked, setChecked] = useState<Checked>({ kind: "checking" });
  const [file, setFile] = useState<SigningFile | null>(null);
  const [password, setPassword] = useState("");
  const [form, setForm] = useState<SignForm>(DEFAULT_FORM);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SignResult | null>(null);
  const ids = {
    password: useId(),
    reason: useId(),
    corner: useId(),
    appearance: useId(),
    error: useId(),
    file: useId(),
  };

  // Every time it opens, it starts over, and the document is checked again: it may have changed.
  const [asked, setAsked] = useState<{ open: boolean; doc: DocumentId | null }>({ open: false, doc: null });
  if (open !== asked.open || (open && doc !== asked.doc)) {
    setAsked({ open, doc: open ? doc : null });
    setFile(null);
    setPassword("");
    setForm(DEFAULT_FORM);
    setError(null);
    setRunning(false);
    setResult(null);
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
  const ready = check === "ready";
  const reasonOk = reasonFits(form.reason);
  const canSign = ready && file !== null && password !== "" && reasonOk && !running;

  const close = () => {
    // The field never keeps a password once the dialog is gone.
    setPassword("");
    onOpenChange(false);
  };

  const chooseFile = () => {
    setError(null);
    api.chooseFile().then(
      (chosen) => {
        // Null: the user closed the system's dialog; the file chosen before stays.
        if (chosen) setFile(chosen);
      },
      () => setError(failure("certificateUnreadable")),
    );
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!canSign || file === null) return;
    setRunning(true);
    setError(null);
    api.sign(argsOf(doc, file.file, password, form, page)).then(
      (done) => {
        setRunning(false);
        // Null: the user closed the system's save dialog; this one stays for another try.
        if (done) {
          setPassword("");
          setResult(done);
        }
      },
      (reason: unknown) => {
        setRunning(false);
        setPassword("");
        setError(failure(errorCodeOf(reason)));
      },
    );
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? onOpenChange(true) : !running && close())}>
      <DialogContent className="max-h-[90vh] overflow-auto sm:max-w-lg">
        {result ? (
          <div className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.resultTitle}</DialogTitle>
              <DialogDescription>{t.title}</DialogDescription>
            </DialogHeader>
            <p role="status" className="text-sm">
              {t.result(result.name)}
            </p>
            <DialogFooter>
              <Button type="button" onClick={close}>
                {t.done}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={submit} className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.title}</DialogTitle>
              <DialogDescription>{t.description}</DialogDescription>
            </DialogHeader>
            {checked.kind === "checking" && (
              <p role="status" className="text-sm">
                {t.checking}
              </p>
            )}
            {checked.kind === "failed" && (
              <p role="alert" className="text-sm text-destructive">
                {t.checkFailed}
              </p>
            )}
            {refusal && (
              <p role="alert" className="text-sm">
                {refusal}
              </p>
            )}
            {ready && (
              <>
                <div className="grid gap-1 text-sm">
                  <span id={ids.file} className="font-medium">
                    {t.fileLabel}
                  </span>
                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      aria-describedby={ids.file}
                      disabled={running}
                      onClick={chooseFile}
                    >
                      {t.chooseFile}
                    </Button>
                    <span className="min-w-0 truncate text-muted-foreground">{file ? file.name : t.noFile}</span>
                  </div>
                </div>
                <div className="grid gap-1 text-sm">
                  <label htmlFor={ids.password} className="font-medium">
                    {t.passwordLabel}
                  </label>
                  <input
                    id={ids.password}
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={error !== null}
                    aria-describedby={error !== null ? ids.error : undefined}
                    value={password}
                    disabled={running}
                    onChange={(event) => setPassword(event.target.value)}
                    className="h-8 rounded-md border bg-background px-2 aria-invalid:border-destructive"
                  />
                </div>
                <fieldset className="grid gap-1 text-sm">
                  <legend className="mb-1 font-medium">{t.appearanceTitle}</legend>
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name={ids.appearance}
                      disabled={running}
                      checked={form.corner !== null}
                      onChange={() => setForm((now) => ({ ...now, corner: DEFAULT_FORM.corner }))}
                    />
                    {t.visible(page)}
                  </label>
                  {form.corner !== null && (
                    <div className="flex items-center gap-2 pl-6">
                      <label htmlFor={ids.corner}>{t.cornerLabel}</label>
                      <select
                        id={ids.corner}
                        value={form.corner}
                        disabled={running}
                        onChange={(event) =>
                          setForm((now) => ({ ...now, corner: event.target.value as SignForm["corner"] }))
                        }
                        className="h-8 rounded-md border bg-background px-2"
                      >
                        {CORNERS.map((corner) => (
                          <option key={corner} value={corner}>
                            {t.corners[corner]}
                          </option>
                        ))}
                      </select>
                    </div>
                  )}
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name={ids.appearance}
                      disabled={running}
                      checked={form.corner === null}
                      onChange={() => setForm((now) => ({ ...now, corner: null }))}
                    />
                    {t.invisible}
                  </label>
                </fieldset>
                <div className="grid gap-1 text-sm">
                  <label htmlFor={ids.reason} className="font-medium">
                    {t.reasonLabel}
                  </label>
                  <input
                    id={ids.reason}
                    type="text"
                    autoComplete="off"
                    aria-invalid={!reasonOk}
                    value={form.reason}
                    disabled={running}
                    onChange={(event) => setForm((now) => ({ ...now, reason: event.target.value }))}
                    className="h-8 rounded-md border bg-background px-2 aria-invalid:border-destructive"
                  />
                  {!reasonOk && <p className="text-destructive">{t.reasonTooLong}</p>}
                </div>
                <p className="text-sm text-muted-foreground">{t.note}</p>
              </>
            )}
            <p role="status" className="min-h-5 text-sm text-muted-foreground">
              {running ? t.running : ""}
            </p>
            {error && (
              <p id={ids.error} role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" disabled={running} onClick={close}>
                {ready ? t.cancel : t.close}
              </Button>
              {ready && (
                <Button type="submit" disabled={!canSign}>
                  {t.start}
                </Button>
              )}
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
