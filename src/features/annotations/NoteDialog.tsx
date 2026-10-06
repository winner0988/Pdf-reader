// What a note says (B2-07): typed for a new note, or changed for one already on the page.

import { useId, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { noteText, noteTooLong } from "@/features/annotations/model";
import { strings } from "@/i18n/zh-TW";

const t = strings.annotations.noteDialog;

type NoteDialogProps = {
  /** A new note, or what an existing one says. */
  initial: string | null;
  onSave: (text: string) => void;
  onCancel: () => void;
};

/** Shown while it is mounted: the shell mounts it for one note and unmounts it when done. */
export function NoteDialog({ initial, onSave, onCancel }: NoteDialogProps) {
  const [text, setText] = useState(initial ?? "");
  const [problem, setProblem] = useState<string | null>(null);
  const fieldId = useId();
  const problemId = useId();

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const cleaned = noteText(text);
    if (cleaned.trim() === "") setProblem(t.empty);
    else if (noteTooLong(cleaned)) setProblem(t.tooLong);
    else onSave(cleaned);
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <form onSubmit={submit} className="grid gap-4" noValidate>
          <DialogHeader>
            <DialogTitle>{initial === null ? t.addTitle : t.editTitle}</DialogTitle>
          </DialogHeader>
          <div className="grid gap-2 text-sm">
            <label htmlFor={fieldId} className="font-medium">
              {t.label}
            </label>
            <textarea
              id={fieldId}
              autoFocus
              rows={5}
              value={text}
              aria-invalid={problem !== null}
              aria-describedby={problem ? problemId : undefined}
              onChange={(event) => {
                setText(event.target.value);
                setProblem(null);
              }}
              className="min-h-24 resize-y rounded-md border bg-background px-2 py-1.5 aria-invalid:border-destructive"
            />
            {problem && (
              <p id={problemId} role="alert" className="text-destructive">
                {problem}
              </p>
            )}
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onCancel}>
              {t.cancel}
            </Button>
            <Button type="submit">{t.save}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
