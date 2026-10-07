// Encrypting a copy of the document (B2-15, docs/architecture/encrypt-copy.md): the passwords and
// restrictions are asked for here, then the main process asks where the copy goes. The passwords go
// to the main process once and nothing keeps them (MVP-16): the fields are emptied as soon as the
// dialog is left or the copy is on its way.

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
import { argsOf, EMPTY_FORM, problemWith, type EncryptForm, type Problem } from "@/features/saving/encrypt";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type DocumentId, type EncryptArgs } from "@/ipc/generated/contract";

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

function describe(problem: Problem): string {
  return problem === "tooLong" ? t.problem.tooLong(LIMITS.maxNewPasswordBytes) : t.problem[problem];
}

export function EncryptDialog({ open, onOpenChange, doc, onEncrypt, onFinished }: EncryptDialogProps) {
  const [form, setForm] = useState<EncryptForm>(EMPTY_FORM);
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);
  const ids = { open: useId(), openAgain: useId(), permissions: useId(), permissionsAgain: useId() };

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

  const field = (id: string, label: string, name: "open" | "openAgain" | "permissions" | "permissionsAgain") => (
    <div className="grid gap-1">
      <label htmlFor={id} className="font-medium">
        {label}
      </label>
      <input
        id={id}
        type="password"
        autoComplete="new-password"
        spellCheck={false}
        disabled={running}
        value={form[name]}
        onChange={(event) => change({ [name]: event.target.value })}
        className="h-8 rounded-md border bg-background px-2"
      />
    </div>
  );

  return (
    <Dialog open={open} onOpenChange={(next) => !running && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-auto sm:max-w-lg">
        <form onSubmit={start} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>
          <fieldset className="grid gap-2 text-sm">
            {field(ids.open, t.openPassword, "open")}
            {field(ids.openAgain, `${t.openPassword}（${t.again}）`, "openAgain")}
            <p className="text-muted-foreground">{t.openPasswordHint}</p>
          </fieldset>
          <fieldset className="grid gap-2 text-sm">
            <legend className="mb-1 font-medium">{t.restrictionsTitle}</legend>
            {(["print", "copy", "modify"] as const).map((kind) => (
              <label key={kind} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  disabled={running}
                  checked={form.restrictions[kind]}
                  onChange={(event) =>
                    change({ restrictions: { ...form.restrictions, [kind]: event.target.checked } })
                  }
                />
                {t.restrict[kind]}
              </label>
            ))}
            {field(ids.permissions, t.permissionsPassword, "permissions")}
            {field(ids.permissionsAgain, `${t.permissionsPassword}（${t.again}）`, "permissionsAgain")}
            <p className="text-muted-foreground">{t.permissionsPasswordHint}</p>
          </fieldset>
          <p className="text-sm text-muted-foreground">{t.note}</p>
          <p className="text-sm text-muted-foreground">{t.signatures}</p>
          <p role="status" className="min-h-5 text-sm text-muted-foreground">
            {running ? t.running : problem !== null ? describe(problem) : ""}
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
