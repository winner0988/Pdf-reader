// Signing a document (B3-06, ADR 0020, docs/architecture/signing.md): whether it can be signed is
// said when the dialog opens; then the certificate (one of this computer's, on a smart card too, or
// a certificate file chosen in the main process's dialog with its password), where the signature
// shows and why. The main process asks where the signed copy goes, verifies it, and opens it in a
// tab of its own; the dialog says so, or why it did not work. A card's PIN is asked for by Windows
// or the card, never here.

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
  dateOf,
  DEFAULT_FORM,
  failure,
  reasonFits,
  REFUSALS,
  usable,
  type SignForm,
  type Source,
} from "@/features/signing/signing";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type {
  DocumentId,
  SignCheck,
  SigningFile,
  SigningKey,
  SignResult,
  StoreCertificate,
} from "@/ipc/generated/contract";

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

/** What the dialog knows of this computer's certificates. */
type Listed = { kind: "listing" } | { kind: "failed" } | { kind: "listed"; certificates: StoreCertificate[] };

export function SignDialog({ open, onOpenChange, doc, page, api }: SignDialogProps) {
  const [checked, setChecked] = useState<Checked>({ kind: "checking" });
  const [listed, setListed] = useState<Listed>({ kind: "listing" });
  const [lists, setLists] = useState(0);
  // `null`: not chosen by the user, so this computer's certificates if one can sign.
  const [source, setSource] = useState<Source | null>(null);
  // `null`: not chosen by the user, so the first one that can sign.
  const [certificate, setCertificate] = useState<number | null>(null);
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
    source: useId(),
    certificate: useId(),
    error: useId(),
    file: useId(),
  };

  // Every time it opens, it starts over, and the document is checked again: it may have changed.
  const [asked, setAsked] = useState<{ open: boolean; doc: DocumentId | null }>({ open: false, doc: null });
  if (open !== asked.open || (open && doc !== asked.doc)) {
    setAsked({ open, doc: open ? doc : null });
    setSource(null);
    setCertificate(null);
    setFile(null);
    setPassword("");
    setForm(DEFAULT_FORM);
    setError(null);
    setRunning(false);
    setResult(null);
    setChecked({ kind: "checking" });
    setListed({ kind: "listing" });
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
  // The store is read when the dialog opens, and again when asked (a card put in since).
  useEffect(() => {
    if (!open) return;
    let current = true;
    api.listCertificates().then(
      (certificates) => current && setListed({ kind: "listed", certificates }),
      () => current && setListed({ kind: "failed" }),
    );
    return () => {
      current = false;
    };
  }, [open, api, lists]);

  const check = checked.kind === "checked" ? checked.check : null;
  const refusal = check === null ? null : (REFUSALS[check] ?? null);
  const ready = check === "ready";
  const reasonOk = reasonFits(form.reason);
  const certificates = listed.kind === "listed" ? listed.certificates : [];
  const signers = usable(certificates);
  const from: Source = source ?? (listed.kind === "listed" && signers.length === 0 ? "file" : "store");
  const chosen = signers.find((listedOne) => listedOne.certificate === certificate) ?? signers[0] ?? null;
  let key: SigningKey | null = null;
  if (from === "store" && chosen !== null) key = { kind: "store", certificate: chosen.certificate };
  if (from === "file" && file !== null && password !== "") key = { kind: "file", file: file.file, password };
  const canSign = ready && key !== null && reasonOk && !running;

  const close = () => {
    // The field never keeps a password once the dialog is gone.
    setPassword("");
    onOpenChange(false);
  };

  const chooseSource = (next: Source) => {
    setError(null);
    setSource(next);
  };

  // Asked for while the user looks at this computer's certificates: they stay on them.
  const refresh = () => {
    setError(null);
    setSource("store");
    setListed({ kind: "listing" });
    setLists((count) => count + 1);
  };

  const chooseFile = () => {
    setError(null);
    api.chooseFile().then(
      (chosenFile) => {
        // Null: the user closed the system's dialog; the file chosen before stays.
        if (chosenFile) setFile(chosenFile);
      },
      () => setError(failure("certificateUnreadable")),
    );
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!canSign || key === null) return;
    setRunning(true);
    setError(null);
    api.sign(argsOf(doc, key, form, page)).then(
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
        // Nothing is said when the user closed the window that asked for the PIN.
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
                <fieldset className="grid gap-1 text-sm">
                  <legend className="mb-1 font-medium">{t.sourceTitle}</legend>
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name={ids.source}
                      disabled={running}
                      checked={from === "store"}
                      onChange={() => chooseSource("store")}
                    />
                    {t.sourceStore}
                  </label>
                  {from === "store" && (
                    <div className="grid gap-2 pb-1 pl-6">
                      {listed.kind === "listing" && (
                        <p role="status" className="text-muted-foreground">
                          {t.listing}
                        </p>
                      )}
                      {listed.kind === "failed" && (
                        <p role="alert" className="text-destructive">
                          {t.listFailed}
                        </p>
                      )}
                      {listed.kind === "listed" && signers.length === 0 && (
                        <p className="text-muted-foreground">{t.noCertificates}</p>
                      )}
                      {certificates.length > 0 && (
                        <ul className="grid gap-2" aria-label={t.sourceStore}>
                          {certificates.map((listedOne) => {
                            const input = `${ids.certificate}-${listedOne.certificate}`;
                            const detail = `${input}-detail`;
                            return (
                              <li key={listedOne.certificate} className="flex items-start gap-2">
                                <input
                                  id={input}
                                  type="radio"
                                  className="mt-1"
                                  name={ids.certificate}
                                  aria-describedby={detail}
                                  disabled={running || listedOne.problem !== null}
                                  checked={chosen?.certificate === listedOne.certificate}
                                  onChange={() => setCertificate(listedOne.certificate)}
                                />
                                <span className="grid min-w-0">
                                  <label htmlFor={input} className="truncate">
                                    {listedOne.subject}
                                  </label>
                                  <span id={detail} className="text-muted-foreground">
                                    {t.certificateDetail(listedOne.issuer, dateOf(listedOne.expires))}
                                    {listedOne.problem && `（${t.problems[listedOne.problem]}）`}
                                  </span>
                                </span>
                              </li>
                            );
                          })}
                        </ul>
                      )}
                      {listed.kind !== "listing" && (
                        <div>
                          <Button type="button" variant="outline" size="sm" disabled={running} onClick={refresh}>
                            {t.refresh}
                          </Button>
                        </div>
                      )}
                      <p className="text-muted-foreground">{t.pinNote}</p>
                    </div>
                  )}
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name={ids.source}
                      disabled={running}
                      checked={from === "file"}
                      onChange={() => chooseSource("file")}
                    />
                    {t.sourceFile}
                  </label>
                  {from === "file" && (
                    <div className="grid gap-2 pl-6">
                      <div className="grid gap-1">
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
                          <span className="min-w-0 truncate text-muted-foreground">
                            {file ? file.name : t.noFile}
                          </span>
                        </div>
                      </div>
                      <div className="grid gap-1">
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
                    </div>
                  )}
                </fieldset>
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
