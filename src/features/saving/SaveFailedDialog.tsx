// Saving failed (B2-02): why, that the changes are still there, and, when it can help, saving a
// copy elsewhere instead.

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

type SaveFailedDialogProps = {
  /** The main process's error; the dialog shows while there is one. */
  failure: unknown;
  /** Offered as a way out; absent when saving a copy would not help. */
  onSaveAs?: () => void;
  onClose: () => void;
};

export function SaveFailedDialog({ failure, onSaveAs, onClose }: SaveFailedDialogProps) {
  const open = failure !== null && failure !== undefined;
  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t.failedTitle}</DialogTitle>
          <DialogDescription>{open ? saveFailure(failure) : ""}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          {onSaveAs && (
            <Button variant="outline" onClick={onSaveAs}>
              {t.saveAs}
            </Button>
          )}
          <Button onClick={onClose}>{t.ok}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
