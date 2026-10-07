// Turning the form into page content (B2-09): asked first, because the fields go for good.

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

const t = strings.forms.flattenDialog;

type FlattenDialogProps = {
  /** Flattens, and then asks where to save the result. */
  onConfirm: () => void;
  onCancel: () => void;
};

/** Shown while it is mounted: the shell mounts it when asked, and unmounts it when done. */
export function FlattenDialog({ onConfirm, onCancel }: FlattenDialogProps) {
  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            {t.cancel}
          </Button>
          <Button autoFocus onClick={onConfirm}>
            {t.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
