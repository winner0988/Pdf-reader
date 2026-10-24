import { AlertTriangle, Paperclip } from "lucide-react";
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
import { strings } from "@/i18n/zh-TW";
import type { AttachmentInfo, DocumentId } from "@/ipc/generated/contract";
import { type AttachmentsApi, tauriAttachmentsApi } from "./api";
import { formatFileSize } from "./size";
import { useAttachments } from "./useAttachments";

export type AttachmentsPanelProps = {
  doc?: DocumentId;
  isSensitive?: boolean;
  api?: AttachmentsApi;
};

export function AttachmentsPanel({
  doc,
  isSensitive = false,
  api = tauriAttachmentsApi,
}: AttachmentsPanelProps) {
  const state = useAttachments(api, doc);
  const [confirmItem, setConfirmItem] = useState<AttachmentInfo | null>(null);
  const [savingId, setSavingId] = useState<number | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [isErrorNotice, setIsErrorNotice] = useState(false);

  const t = strings.attachments;

  const performSave = async (item: AttachmentInfo) => {
    if (doc === undefined) return;
    setSavingId(item.id);
    setNotice(null);
    try {
      const result = await api.saveAttachment(doc, item.id);
      if (result) {
        if (!result.marked) {
          setIsErrorNotice(true);
          setNotice(t.notMarked);
        } else {
          setIsErrorNotice(false);
          setNotice(t.saveSuccess);
        }
      }
    } catch {
      setIsErrorNotice(true);
      setNotice(t.saveFailed);
    } finally {
      setSavingId(null);
    }
  };

  const handleSaveClick = (item: AttachmentInfo) => {
    if (item.isDangerous) {
      setConfirmItem(item);
    } else {
      void performSave(item);
    }
  };

  if (state.status === "loading") {
    return <p className="px-2 py-4 text-sm text-muted-foreground">{t.loading}</p>;
  }

  if (state.status === "failed") {
    return <p className="px-2 py-4 text-sm text-muted-foreground">{t.failed}</p>;
  }

  if (state.items.length === 0) {
    return <p className="px-2 py-4 text-sm text-muted-foreground">{t.empty}</p>;
  }

  return (
    <div className="flex flex-col gap-3 py-2" data-testid="attachments-panel">
      <div className="flex items-center justify-between px-1">
        <span className="text-xs font-medium text-muted-foreground">{t.count(state.items.length)}</span>
      </div>

      {isSensitive && (
        <div
          role="alert"
          data-testid="sensitive-warning"
          className="rounded border border-amber-500/40 bg-amber-500/10 p-2 text-xs text-amber-800 dark:text-amber-200"
        >
          {t.sensitiveWarning}
        </div>
      )}

      {notice && (
        <div
          role="status"
          data-testid="attachments-notice"
          className={`rounded border p-2 text-xs ${
            isErrorNotice
              ? "border-destructive/40 bg-destructive/10 text-destructive"
              : "border-primary/40 bg-primary/10 text-primary"
          }`}
        >
          {notice}
        </div>
      )}

      <ul className="flex flex-col gap-2" role="list">
        {state.items.map((item) => {
          const isSaving = savingId === item.id;
          const extDisplay = item.extension || t.noExtension;
          const sizeText = item.size !== null ? t.claimedSize(formatFileSize(item.size)) : t.unknownSize;
          const sourceText =
            item.source.type === "document" ? t.sourceDocument : t.sourcePage(item.source.page + 1);

          return (
            <li
              key={item.id}
              data-testid="attachment-item"
              className="flex flex-col gap-1.5 rounded-lg border bg-card p-2.5 text-card-foreground shadow-sm"
            >
              <div className="flex items-start justify-between gap-2">
                <div className="flex min-w-0 items-start gap-1.5">
                  <Paperclip className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <div className="min-w-0">
                    <p className="break-all text-xs font-medium leading-snug" title={item.name}>
                      {item.name}
                    </p>
                    <div className="mt-1 flex flex-wrap items-center gap-1.5">
                      <span
                        data-testid="attachment-extension"
                        className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground"
                      >
                        {extDisplay}
                      </span>
                      <span className="text-[11px] text-muted-foreground">{sourceText}</span>
                      <span className="text-[11px] text-muted-foreground">·</span>
                      <span className="text-[11px] text-muted-foreground">{sizeText}</span>
                    </div>
                  </div>
                </div>
              </div>

              {item.description && (
                <p className="line-clamp-2 text-xs text-muted-foreground" title={item.description}>
                  {item.description}
                </p>
              )}

              {item.isDangerous && (
                <div
                  data-testid="attachment-dangerous"
                  className="flex items-center gap-1 text-[11px] font-medium text-destructive"
                >
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                  <span>{t.dangerousWarning}</span>
                </div>
              )}

              <div className="mt-1 flex justify-end">
                <Button
                  size="sm"
                  variant="outline"
                  data-testid="save-attachment-button"
                  disabled={isSaving}
                  onClick={() => handleSaveClick(item)}
                  className="h-7 text-xs"
                >
                  {isSaving ? t.saving : t.save}
                </Button>
              </div>
            </li>
          );
        })}
      </ul>

      {confirmItem && (
        <Dialog open={true} onOpenChange={(open) => !open && setConfirmItem(null)}>
          <DialogContent data-testid="danger-confirm-dialog">
            <DialogHeader>
              <DialogTitle>{t.dangerConfirm.title}</DialogTitle>
              <DialogDescription>
                {t.dangerConfirm.message(confirmItem.extension || t.noExtension)}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter className="gap-2 sm:gap-0">
              <Button
                variant="outline"
                data-testid="danger-cancel-button"
                onClick={() => setConfirmItem(null)}
              >
                {t.dangerConfirm.cancel}
              </Button>
              <Button
                variant="destructive"
                data-testid="danger-confirm-button"
                onClick={() => {
                  const target = confirmItem;
                  setConfirmItem(null);
                  void performSave(target);
                }}
              >
                {t.dangerConfirm.confirm}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
    </div>
  );
}
