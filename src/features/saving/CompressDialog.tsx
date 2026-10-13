// A copy of the document with its pictures encoded again, smaller (B3-01,
// docs/architecture/compress-images.md): the quality and the size are asked for here, then the main
// process asks where the copy goes. What came of it is said in the dialog itself, which stays until
// the user is done reading it: how big the copy is and how many pictures were made smaller.

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
import {
  argsOf,
  DEFAULT_FORM,
  describe,
  QUALITIES,
  SIDES,
  type CompressForm,
} from "@/features/saving/compress";
import { strings } from "@/i18n/zh-TW";
import type { CompressArgs, CompressResult, DocumentId } from "@/ipc/generated/contract";

const t = strings.compress;

type CompressDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  /** Writes the copy: `null` if the user closed the system's save dialog. */
  onCompress: (args: CompressArgs) => Promise<CompressResult | null>;
};

export function CompressDialog({ open, onOpenChange, doc, onCompress }: CompressDialogProps) {
  const [form, setForm] = useState<CompressForm>(DEFAULT_FORM);
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);
  const [result, setResult] = useState<CompressResult | null>(null);
  const names = { quality: useId(), size: useId() };

  // Every time it opens, it starts over.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    setForm(DEFAULT_FORM);
    setFailed(false);
    setResult(null);
  }

  const start = (event: FormEvent) => {
    event.preventDefault();
    if (running) return;
    setRunning(true);
    setFailed(false);
    onCompress(argsOf(doc, form)).then(
      (done) => {
        setRunning(false);
        // Null: the user closed the system's dialog; this one stays for another try.
        if (done) setResult(done);
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
        {result ? (
          <div className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.resultTitle}</DialogTitle>
              <DialogDescription>{t.title}</DialogDescription>
            </DialogHeader>
            <p role="status" className="text-sm">
              {describe(result)}
            </p>
            <DialogFooter>
              <Button type="button" onClick={() => onOpenChange(false)}>
                {t.close}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={start} className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.title}</DialogTitle>
              <DialogDescription>{t.description}</DialogDescription>
            </DialogHeader>
            <fieldset className="grid gap-1 text-sm">
              <legend className="mb-1 font-medium">{t.qualityTitle}</legend>
              {QUALITIES.map((quality) => (
                <label key={quality} className="flex items-center gap-2">
                  <input
                    type="radio"
                    name={names.quality}
                    disabled={running}
                    checked={form.quality === quality}
                    onChange={() => setForm((now) => ({ ...now, quality }))}
                  />
                  {t.quality[quality]}
                </label>
              ))}
            </fieldset>
            <fieldset className="grid gap-1 text-sm">
              <legend className="mb-1 font-medium">{t.sizeTitle}</legend>
              {SIDES.map((side) => (
                <label key={side ?? "none"} className="flex items-center gap-2">
                  <input
                    type="radio"
                    name={names.size}
                    disabled={running}
                    checked={form.maxSide === side}
                    onChange={() => setForm((now) => ({ ...now, maxSide: side }))}
                  />
                  {side === null ? t.sizeNone : t.size(side)}
                </label>
              ))}
            </fieldset>
            <p className="text-sm text-muted-foreground">{t.note}</p>
            <p className="text-sm text-muted-foreground">{t.signatures}</p>
            <p role="status" className="min-h-5 text-sm text-muted-foreground">
              {running ? t.running : ""}
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
              <Button type="submit" disabled={running}>
                {t.start}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
