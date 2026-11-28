// Dialog for managing and placing saved signature images (B6-02).

import { useCallback, useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SignatureCreator } from "@/features/annotations/SignatureCreator";
import type { SignaturesApi } from "@/features/annotations/signaturesApi";
import { useSignaturePreviews } from "@/features/annotations/useSignaturePreviews";
import { strings } from "@/i18n";
import {
  type DocumentId,
  LIMITS,
  type SignatureImage,
  type SignatureImageId,
  type StampImageInfo,
} from "@/ipc/generated/contract";

export type SignaturesDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  api: SignaturesApi;
  doc?: DocumentId;
  onSelect?: (picture: StampImageInfo) => void;
  onError?: (error: unknown) => void;
  toPngBytes?: (canvas: HTMLCanvasElement) => Promise<number[]>;
};

export function SignaturesDialog({
  open,
  onOpenChange,
  api,
  doc,
  onSelect,
  onError,
  toPngBytes,
}: SignaturesDialogProps) {
  const t = strings.savedSignatures;
  const titleId = useId();
  const descId = useId();

  const [signatures, setSignatures] = useState<SignatureImage[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [deletingId, setDeletingId] = useState<SignatureImageId | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const previews = useSignaturePreviews(open, signatures, api.getSignatureImagePng);

  // Load signatures on open
  const loadSignatures = useCallback(() => {
    if (!open) return;
    api.getSignatureImages().then(
      (list) => {
        setSignatures(list);
        setErrorMsg(null);
      },
      () => setErrorMsg(t.loadFailed),
    );
  }, [api, open, t.loadFailed]);

  useEffect(() => {
    loadSignatures();
  }, [loadSignatures]);

  const maxReached = (signatures?.length ?? 0) >= LIMITS.maxSignatureImages;

  // Place signature on current document
  const handleSelect = async (id: SignatureImageId) => {
    if (doc === undefined || busy) return;
    setBusy(true);
    setNotice(null);
    try {
      const picture = await api.prepareSignatureStamp(doc, id);
      onSelect?.(picture);
      onOpenChange(false);
    } catch (err) {
      onError?.(err);
      setErrorMsg(t.failed);
    } finally {
      setBusy(false);
    }
  };

  // Delete individual signature
  const handleDelete = async (id: SignatureImageId) => {
    setBusy(true);
    setNotice(null);
    try {
      const remaining = await api.removeSignatureImage(id);
      setSignatures(remaining);
      setDeletingId(null);
    } catch {
      setErrorMsg(t.deleteFailed);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle id={titleId}>{t.dialogTitle}</DialogTitle>
          <DialogDescription id={descId}>{t.dialogDescription}</DialogDescription>
        </DialogHeader>

        {/* Security / local storage notice with role="note" */}
        <div role="note" className="rounded border bg-muted/40 p-2.5 text-xs text-muted-foreground space-y-1">
          <p>{t.note}</p>
        </div>

        {errorMsg && (
          <p role="alert" className="text-xs text-destructive">
            {errorMsg}
          </p>
        )}
        {notice && <p className="text-xs text-muted-foreground">{notice}</p>}

        {/* Saved signatures list */}
        <div className="space-y-2">
          <h3 className="text-sm font-medium">{t.savedListTitle}</h3>
          {signatures === null ? (
            <p className="text-xs text-muted-foreground">...</p>
          ) : signatures.length === 0 ? (
            <p className="text-xs text-muted-foreground">{t.noSignatures}</p>
          ) : (
            <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
              {signatures.map((sig) => {
                const previewUrl = previews[sig.id];
                const isConfirming = deletingId === sig.id;
                return (
                  <div
                    key={sig.id}
                    className="group relative flex flex-col items-center justify-between rounded border bg-card p-2 hover:border-primary transition-colors"
                  >
                    <button
                      type="button"
                      disabled={doc === undefined || busy}
                      onClick={() => handleSelect(sig.id)}
                      className="flex h-20 w-full items-center justify-center p-1 cursor-pointer focus:outline-none focus-visible:ring-2 focus-visible:ring-primary rounded"
                      title={t.clickToPlace}
                      aria-label={`${t.clickToPlace} (${sig.id})`}
                    >
                      {previewUrl ? (
                        <img
                          src={previewUrl}
                          alt=""
                          className="max-h-full max-w-full object-contain"
                        />
                      ) : (
                        <span className="text-xs text-muted-foreground"># {sig.id}</span>
                      )}
                    </button>
                    <div className="mt-1 flex w-full justify-end">
                      {isConfirming ? (
                        <div className="flex items-center gap-1">
                          <Button
                            variant="destructive"
                            size="sm"
                            className="h-6 px-1.5 text-xs"
                            disabled={busy}
                            onClick={() => handleDelete(sig.id)}
                          >
                            {t.confirmDelete}
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            className="h-6 px-1.5 text-xs"
                            disabled={busy}
                            onClick={() => setDeletingId(null)}
                          >
                            {t.cancelDelete}
                          </Button>
                        </div>
                      ) : (
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-6 px-2 text-xs text-muted-foreground hover:text-destructive"
                          disabled={busy}
                          onClick={() => setDeletingId(sig.id)}
                        >
                          {t.delete}
                        </Button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Max signatures warning */}
        {maxReached && (
          <p role="status" className="rounded bg-destructive/10 p-2 text-xs font-medium text-destructive">
            {t.maxReached}
          </p>
        )}

        {/* Add new signature tabs */}
        <div className="border-t pt-3">
          <SignatureCreator
            api={api}
            disabled={maxReached}
            busy={busy}
            onBusyChange={setBusy}
            onBeforeAction={() => {
              setNotice(null);
              setErrorMsg(null);
            }}
            onCreated={setSignatures}
            onError={setErrorMsg}
            toPngBytes={toPngBytes}
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
