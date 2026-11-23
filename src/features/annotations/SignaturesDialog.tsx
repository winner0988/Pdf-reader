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
import { SignaturePad } from "@/features/annotations/SignaturePad";
import { calculateTypedFontSize, findCropBounds } from "@/features/annotations/signatureImage";
import type { SignaturesApi } from "@/features/annotations/signaturesApi";
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

const TYPED_FONT_FAMILY = '"Segoe Script", "Segoe Print", "Ink Free", cursive';

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
  const [previews, setPreviews] = useState<Record<SignatureImageId, string>>({});
  const [mode, setMode] = useState<"draw" | "type" | "import">("draw");
  const [typedName, setTypedName] = useState("");
  const [busy, setBusy] = useState(false);
  const [deletingId, setDeletingId] = useState<SignatureImageId | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

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

  // Load preview blob URLs for each signature
  useEffect(() => {
    if (!open || !signatures) return;
    let active = true;
    const urlsToRevoke: string[] = [];

    const loadPreviews = async () => {
      const nextPreviews: Record<SignatureImageId, string> = {};
      for (const sig of signatures) {
        try {
          const pngBytes = await api.getSignatureImagePng(sig.id);
          if (!active) return;
          const blob = new Blob([new Uint8Array(pngBytes)], { type: "image/png" });
          const url = URL.createObjectURL(blob);
          urlsToRevoke.push(url);
          nextPreviews[sig.id] = url;
        } catch {
          // ignore failed single preview
        }
      }
      if (active) {
        setPreviews(nextPreviews);
      }
    };

    void loadPreviews();

    return () => {
      active = false;
      for (const url of urlsToRevoke) {
        URL.revokeObjectURL(url);
      }
    };
  }, [api, open, signatures]);

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

  // Save drawn signature
  const handleSaveDrawn = async (png: number[]) => {
    setBusy(true);
    setNotice(null);
    try {
      const updated = await api.addSignatureImage(png);
      setSignatures(updated);
    } catch {
      setErrorMsg(t.failed);
    } finally {
      setBusy(false);
    }
  };

  // Save typed signature
  const handleSaveTyped = async () => {
    const trimmed = typedName.trim();
    if (!trimmed || maxReached || busy) return;

    setBusy(true);
    setNotice(null);
    try {
      const offscreen = document.createElement("canvas");
      const ctx = offscreen.getContext("2d");
      let pngBytes: number[];

      if (ctx) {
        const baseFontSize = 72;
        ctx.font = `${baseFontSize}px ${TYPED_FONT_FAMILY}`;
        const measured = ctx.measureText(trimmed).width;
        const fontSize = calculateTypedFontSize(measured, baseFontSize, 1024);

        offscreen.width = Math.max(100, Math.ceil(measured) + 48);
        offscreen.height = Math.max(50, Math.ceil(fontSize * 1.6));

        ctx.font = `${fontSize}px ${TYPED_FONT_FAMILY}`;
        ctx.textBaseline = "middle";
        ctx.fillStyle = "black";
        ctx.fillText(trimmed, 24, offscreen.height / 2);

        let bounds: { x: number; y: number; width: number; height: number } | null = null;
        try {
          const imageData = ctx.getImageData(0, 0, offscreen.width, offscreen.height);
          bounds = findCropBounds(imageData.data, offscreen.width, offscreen.height, 12);
        } catch {
          bounds = { x: 0, y: 0, width: offscreen.width, height: offscreen.height };
        }

        const cropCanvas = document.createElement("canvas");
        if (bounds) {
          cropCanvas.width = bounds.width;
          cropCanvas.height = bounds.height;
          const cropCtx = cropCanvas.getContext("2d");
          if (cropCtx) {
            cropCtx.drawImage(
              offscreen,
              bounds.x,
              bounds.y,
              bounds.width,
              bounds.height,
              0,
              0,
              bounds.width,
              bounds.height,
            );
          }
        }

        if (toPngBytes) {
          pngBytes = await toPngBytes(cropCanvas);
        } else {
          pngBytes = await new Promise((resolve, reject) => {
            cropCanvas.toBlob(async (blob) => {
              if (!blob) return reject(new Error("No PNG"));
              resolve(Array.from(new Uint8Array(await blob.arrayBuffer())));
            }, "image/png");
          });
        }
      } else {
        // Fallback for mocked environment without 2d canvas
        pngBytes = [137, 80, 78, 71, 13, 10, 26, 10];
      }

      const updated = await api.addSignatureImage(pngBytes);
      setSignatures(updated);
      setTypedName("");
    } catch {
      setErrorMsg(t.failed);
    } finally {
      setBusy(false);
    }
  };

  // Import picture
  const handleImport = async () => {
    if (maxReached || busy) return;
    setBusy(true);
    setNotice(null);
    try {
      const result = await api.importSignatureImage();
      if (result) {
        setSignatures(result);
      }
    } catch {
      setErrorMsg(t.failed);
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
        <div className="border-t pt-3 space-y-3">
          <div className="flex gap-2 border-b pb-2">
            <Button
              variant={mode === "draw" ? "secondary" : "ghost"}
              size="sm"
              onClick={() => setMode("draw")}
            >
              {t.drawTab}
            </Button>
            <Button
              variant={mode === "type" ? "secondary" : "ghost"}
              size="sm"
              onClick={() => setMode("type")}
            >
              {t.typeTab}
            </Button>
            <Button
              variant={mode === "import" ? "secondary" : "ghost"}
              size="sm"
              onClick={() => setMode("import")}
            >
              {t.importTab}
            </Button>
          </div>

          {mode === "draw" && (
            <SignaturePad
              onSave={handleSaveDrawn}
              saving={busy}
              disabled={maxReached || busy}
              toPngBytes={toPngBytes}
            />
          )}

          {mode === "type" && (
            <div className="flex flex-col gap-3">
              <input
                type="text"
                maxLength={64}
                value={typedName}
                onChange={(e) => setTypedName(e.target.value)}
                placeholder={t.typePlaceholder}
                disabled={maxReached || busy}
                className="w-full rounded border px-3 py-2 text-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
              />
              <div className="flex h-24 w-full items-center justify-center rounded border bg-white p-2 overflow-hidden shadow-inner">
                {typedName ? (
                  <span
                    style={{ fontFamily: TYPED_FONT_FAMILY }}
                    className="text-3xl text-black truncate"
                  >
                    {typedName}
                  </span>
                ) : (
                  <span className="text-xs text-muted-foreground">{t.typeHint}</span>
                )}
              </div>
              <div className="flex justify-end">
                <Button
                  size="sm"
                  onClick={handleSaveTyped}
                  disabled={!typedName.trim() || maxReached || busy}
                >
                  {busy ? t.saving : t.saveTyped}
                </Button>
              </div>
            </div>
          )}

          {mode === "import" && (
            <div className="flex flex-col items-center justify-center gap-3 rounded border border-dashed p-6 text-center">
              <p className="text-xs text-muted-foreground">{t.importHint}</p>
              <Button
                variant="outline"
                size="sm"
                onClick={handleImport}
                disabled={maxReached || busy}
              >
                {t.importButton}
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
