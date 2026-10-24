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

const t = strings.sidebar;

export type DeleteBookmarkDialogProps = {
  open: boolean;
  onConfirm: () => void;
  onCancel: () => void;
};

export function DeleteBookmarkDialog({
  open,
  onConfirm,
  onCancel,
}: DeleteBookmarkDialogProps) {
  return (
    <Dialog open={open} onOpenChange={(next) => (!next ? onCancel() : undefined)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t.deleteConfirmTitle}</DialogTitle>
          <DialogDescription>{t.deleteConfirmDescription}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            {t.deleteConfirmCancel}
          </Button>
          <Button variant="destructive" onClick={onConfirm}>
            {t.deleteConfirmConfirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
