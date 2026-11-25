// The file whose pages are to go into the document defines layers (#703): MuPDF grafts
// pages without Catalog /OCProperties, so all layers in the inserted pages become visible.

import { Layers } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { strings } from "@/i18n";

const t = strings.pages.sourceLayers;

export type SourceLayersDialogProps = {
  open: boolean;
  onConfirm: () => void;
  onCancel: () => void;
};

/** Warning dialog shown before inserting pages from a PDF that defines layers (#703). */
export function SourceLayersDialog({ open, onConfirm, onCancel }: SourceLayersDialogProps) {
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          onCancel();
        }
      }}
    >
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Layers className="size-5 shrink-0" aria-hidden />
            {t.title}
          </DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>

        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-2.5 text-xs text-amber-900 dark:text-amber-200">
          <p>{t.detail}</p>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={onCancel}>
            {t.cancel}
          </Button>
          <Button type="button" onClick={onConfirm}>
            {t.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
