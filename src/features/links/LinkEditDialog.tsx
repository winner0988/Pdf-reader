// Dialog for adding or editing a link target (Issue #588, B6-03):
// points either to a page in the document (1-based in UI, 0-based in IPC)
// or to a web/email address (http, https, or mailto).

import { useId, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { strings } from "@/i18n";
import type { NewLinkTarget } from "@/ipc/generated/contract";
import { validateLinkPage, validateLinkUri } from "./validation";

export type LinkEditDialogProps = {
  mode: "add" | "edit";
  pageCount: number;
  initialTarget?: NewLinkTarget;
  onConfirm: (target: NewLinkTarget) => void;
  onDelete?: () => void;
  onCancel: () => void;
};

export function LinkEditDialog({
  mode,
  pageCount,
  initialTarget,
  onConfirm,
  onDelete,
  onCancel,
}: LinkEditDialogProps) {
  const t = strings.links;
  const initialType = initialTarget?.kind === "uri" ? "uri" : "page";
  const [targetType, setTargetType] = useState<"page" | "uri">(initialType);

  const initialPage =
    initialTarget?.kind === "page" ? String(initialTarget.page + 1) : "1";
  const initialUri = initialTarget?.kind === "uri" ? initialTarget.uri : "https://";

  const [pageInput, setPageInput] = useState(initialPage);
  const [uriInput, setUriInput] = useState(initialUri);
  const [error, setError] = useState<string | null>(null);

  const pageId = useId();
  const uriId = useId();

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault();
    if (targetType === "page") {
      const err = validateLinkPage(pageInput, pageCount);
      if (err) {
        setError(err);
        return;
      }
      const pageNum = parseInt(pageInput.trim(), 10);
      onConfirm({ kind: "page", page: pageNum - 1 });
    } else {
      const err = validateLinkUri(uriInput);
      if (err) {
        setError(err);
        return;
      }
      onConfirm({ kind: "uri", uri: uriInput.trim() });
    }
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onCancel()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{mode === "add" ? t.addTitle : t.editTitle}</DialogTitle>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4">
          <Tabs
            value={targetType}
            onValueChange={(val) => {
              setTargetType(val as "page" | "uri");
              setError(null);
            }}
          >
            <TabsList className="grid w-full grid-cols-2">
              <TabsTrigger value="page">{t.targetPage}</TabsTrigger>
              <TabsTrigger value="uri">{t.targetUri}</TabsTrigger>
            </TabsList>

            <TabsContent value="page" className="space-y-2 pt-3">
              <label htmlFor={pageId} className="text-sm font-medium">
                {t.pageNumber}
              </label>
              <input
                id={pageId}
                type="number"
                min={1}
                max={pageCount}
                value={pageInput}
                onChange={(e) => {
                  setPageInput(e.target.value);
                  setError(null);
                }}
                placeholder={t.pageNumberPlaceholder(pageCount)}
                className="w-full rounded-md border bg-background px-3 py-2 text-sm"
                autoFocus
              />
            </TabsContent>

            <TabsContent value="uri" className="space-y-2 pt-3">
              <label htmlFor={uriId} className="text-sm font-medium">
                {t.uriLabel}
              </label>
              <input
                id={uriId}
                type="text"
                value={uriInput}
                onChange={(e) => {
                  setUriInput(e.target.value);
                  setError(null);
                }}
                placeholder={t.uriPlaceholder}
                className="w-full rounded-md border bg-background px-3 py-2 text-sm"
                autoFocus
              />
            </TabsContent>
          </Tabs>

          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}

          <DialogFooter className="flex items-center justify-between sm:justify-between">
            {mode === "edit" && onDelete ? (
              <Button
                type="button"
                variant="destructive"
                onClick={onDelete}
                data-delete-link-button=""
              >
                {t.delete}
              </Button>
            ) : (
              <div />
            )}
            <div className="flex gap-2">
              <Button type="button" variant="outline" onClick={onCancel}>
                {t.cancel}
              </Button>
              <Button type="submit" data-confirm-link-button="">
                {mode === "add" ? t.add : t.save}
              </Button>
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
