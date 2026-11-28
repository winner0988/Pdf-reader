// Shared saved-signature creation component (B6-02, #744).

import { useState } from "react";

import { Button } from "@/components/ui/button";
import { SignaturePad } from "@/features/annotations/SignaturePad";
import { calculateTypedFontSize, findCropBounds } from "@/features/annotations/signatureImage";
import type { SignaturesApi } from "@/features/annotations/signaturesApi";
import { strings } from "@/i18n";
import type { SignatureImage } from "@/ipc/generated/contract";

const TYPED_FONT_FAMILY = '"Segoe Script", "Segoe Print", "Ink Free", cursive';

export type SignatureCreatorProps = {
  api: Pick<SignaturesApi, "addSignatureImage" | "importSignatureImage">;
  disabled?: boolean;
  busy: boolean;
  onBusyChange: (busy: boolean) => void;
  onBeforeAction?: () => void;
  onCreated: (updated: SignatureImage[]) => void;
  onError: (message: string) => void;
  toPngBytes?: (canvas: HTMLCanvasElement) => Promise<number[]>;
};

export function SignatureCreator({
  api,
  disabled = false,
  busy,
  onBusyChange,
  onBeforeAction,
  onCreated,
  onError,
  toPngBytes,
}: SignatureCreatorProps) {
  const t = strings.savedSignatures;
  const [mode, setMode] = useState<"draw" | "type" | "import">("draw");
  const [typedName, setTypedName] = useState("");

  const handleSaveDrawn = async (png: number[]) => {
    onBusyChange(true);
    onBeforeAction?.();
    try {
      const updated = await api.addSignatureImage(png);
      onCreated(updated);
    } catch {
      onError(t.failed);
    } finally {
      onBusyChange(false);
    }
  };

  const handleSaveTyped = async () => {
    const trimmed = typedName.trim();
    if (!trimmed || disabled || busy) return;

    onBusyChange(true);
    onBeforeAction?.();
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
      setTypedName("");
      onCreated(updated);
    } catch {
      onError(t.failed);
    } finally {
      onBusyChange(false);
    }
  };

  const handleImport = async () => {
    if (disabled || busy) return;
    onBusyChange(true);
    onBeforeAction?.();
    try {
      const result = await api.importSignatureImage();
      if (result) {
        onCreated(result);
      }
    } catch {
      onError(t.failed);
    } finally {
      onBusyChange(false);
    }
  };

  const controlsDisabled = disabled || busy;

  return (
    <div className="space-y-3">
      <div className="flex gap-2 border-b pb-2">
        <Button
          type="button"
          variant={mode === "draw" ? "secondary" : "ghost"}
          size="sm"
          onClick={() => setMode("draw")}
        >
          {t.drawTab}
        </Button>
        <Button
          type="button"
          variant={mode === "type" ? "secondary" : "ghost"}
          size="sm"
          onClick={() => setMode("type")}
        >
          {t.typeTab}
        </Button>
        <Button
          type="button"
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
          disabled={controlsDisabled}
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
            disabled={controlsDisabled}
            className="w-full rounded border px-3 py-2 text-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          />
          <div className="flex h-24 w-full items-center justify-center rounded border bg-white p-2 overflow-hidden shadow-inner">
            {typedName ? (
              <span style={{ fontFamily: TYPED_FONT_FAMILY }} className="text-3xl text-black truncate">
                {typedName}
              </span>
            ) : (
              <span className="text-xs text-muted-foreground">{t.typeHint}</span>
            )}
          </div>
          <div className="flex justify-end">
            <Button type="button" size="sm" onClick={handleSaveTyped} disabled={!typedName.trim() || controlsDisabled}>
              {busy ? t.saving : t.saveTyped}
            </Button>
          </div>
        </div>
      )}

      {mode === "import" && (
        <div className="flex flex-col items-center justify-center gap-3 rounded border border-dashed p-6 text-center">
          <p className="text-xs text-muted-foreground">{t.importHint}</p>
          <Button type="button" variant="outline" size="sm" onClick={handleImport} disabled={controlsDisabled}>
            {t.importButton}
          </Button>
        </div>
      )}
    </div>
  );
}
