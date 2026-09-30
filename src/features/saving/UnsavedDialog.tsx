// Asks what to do with unsaved changes before a tab or the window closes (B2-02): save, don't
// save, or keep it open.

import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { saveFailure } from "@/features/saving/messages";
import { strings } from "@/i18n/zh-TW";

const t = strings.saving;

type UnsavedDialogProps = {
  /** File names of the documents with unsaved changes; the dialog shows while there are any. */
  names: string[] | null;
  /** Saves them all; rejects with the main process's error if one could not be saved. */
  onSave: () => Promise<void>;
  onDiscard: () => void;
  onCancel: () => void;
};

export function UnsavedDialog({ names, onSave, onDiscard, onCancel }: UnsavedDialogProps) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const open = names !== null && names.length > 0;
  const single = names?.length === 1 ? names[0] : undefined;

  const save = () => {
    setSaving(true);
    setError(null);
    onSave().then(
      () => setSaving(false),
      (failure: unknown) => {
        setSaving(false);
        setError(saveFailure(failure));
      },
    );
  };
  const close = (next: boolean) => {
    if (next || saving) return;
    setError(null);
    onCancel();
  };

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t.askTitle}</DialogTitle>
          <DialogDescription>{single !== undefined ? t.askOne(single) : t.askMany(names?.length ?? 0)}</DialogDescription>
        </DialogHeader>
        {single === undefined && names && (
          <ul className="list-disc pl-5 text-sm">
            {names.map((name, index) => (
              <li key={index}>{name}</li>
            ))}
          </ul>
        )}
        {saving && (
          <p role="status" className="text-sm text-muted-foreground">
            {t.saving}
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={saving} onClick={() => close(false)}>
            {t.cancel}
          </Button>
          <Button
            variant="outline"
            disabled={saving}
            onClick={() => {
              setError(null);
              onDiscard();
            }}
          >
            {t.discard}
          </Button>
          <Button disabled={saving} onClick={save}>
            {single !== undefined ? t.save : t.saveAll}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
