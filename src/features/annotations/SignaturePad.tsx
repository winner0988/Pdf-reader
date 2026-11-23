// Signature drawing pad with stroke rendering, auto-crop, and PNG export (B6-02).

import { useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { defaultCanvasToPng, findCropBounds } from "@/features/annotations/signatureImage";
import { strings } from "@/i18n";

export type SignaturePadProps = {
  onSave: (png: number[]) => Promise<void> | void;
  saving?: boolean;
  disabled?: boolean;
  toPngBytes?: (canvas: HTMLCanvasElement) => Promise<number[]>;
};

export function SignaturePad({
  onSave,
  saving = false,
  disabled = false,
  toPngBytes = defaultCanvasToPng,
}: SignaturePadProps) {
  const t = strings.savedSignatures;
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [drawing, setDrawing] = useState(false);
  const [hasDrawn, setHasDrawn] = useState(false);
  const lastPointRef = useRef<{ x: number; y: number } | null>(null);

  const getCanvasPoint = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return { x: 0, y: 0 };
    const rect = canvas.getBoundingClientRect();
    const scaleX = rect.width > 0 ? canvas.width / rect.width : 1;
    const scaleY = rect.height > 0 ? canvas.height / rect.height : 1;
    return {
      x: (e.clientX - rect.left) * scaleX,
      y: (e.clientY - rect.top) * scaleY,
    };
  };

  const startDrawing = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (disabled || saving) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    try {
      canvas.setPointerCapture(e.pointerId);
    } catch {
      // ignore
    }
    setDrawing(true);
    const pt = getCanvasPoint(e);
    lastPointRef.current = pt;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.beginPath();
    ctx.arc(pt.x, pt.y, 3, 0, Math.PI * 2);
    ctx.fillStyle = "black";
    ctx.fill();
    setHasDrawn(true);
  };

  const draw = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (!drawing || disabled || saving) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const currentPt = getCanvasPoint(e);
    const lastPt = lastPointRef.current ?? currentPt;

    ctx.strokeStyle = "black";
    ctx.lineWidth = 6;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(lastPt.x, lastPt.y);
    ctx.lineTo(currentPt.x, currentPt.y);
    ctx.stroke();

    lastPointRef.current = currentPt;
    setHasDrawn(true);
  };

  const stopDrawing = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (drawing) {
      setDrawing(false);
      lastPointRef.current = null;
      try {
        canvasRef.current?.releasePointerCapture(e.pointerId);
      } catch {
        // ignore
      }
    }
  };

  const clear = () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    setHasDrawn(false);
  };

  const handleSave = async () => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let bounds: { x: number; y: number; width: number; height: number } | null;
    try {
      const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);
      bounds = findCropBounds(imageData.data, canvas.width, canvas.height, 12);
    } catch {
      bounds = { x: 0, y: 0, width: canvas.width, height: canvas.height };
    }

    if (!bounds) return;

    const cropCanvas = document.createElement("canvas");
    cropCanvas.width = bounds.width;
    cropCanvas.height = bounds.height;
    const cropCtx = cropCanvas.getContext("2d");
    if (cropCtx) {
      cropCtx.drawImage(
        canvas,
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

    const png = await toPngBytes(cropCanvas);
    await onSave(png);
  };

  return (
    <div className="flex flex-col gap-2">
      <div className="relative border rounded bg-white overflow-hidden shadow-inner">
        <canvas
          ref={canvasRef}
          width={900}
          height={300}
          aria-label={t.padAriaLabel}
          aria-description={t.padKeyboardHint}
          tabIndex={0}
          className="w-full max-w-[600px] h-[200px] touch-none cursor-crosshair block mx-auto bg-transparent"
          onPointerDown={startDrawing}
          onPointerMove={draw}
          onPointerUp={stopDrawing}
          onPointerCancel={stopDrawing}
        />
      </div>
      <p className="text-xs text-muted-foreground">{t.padKeyboardHint}</p>
      <div className="flex items-center justify-end gap-2 pt-1">
        <Button variant="outline" size="sm" onClick={clear} disabled={!hasDrawn || disabled || saving}>
          {t.clearPad}
        </Button>
        <Button size="sm" onClick={handleSave} disabled={!hasDrawn || disabled || saving}>
          {saving ? t.saving : t.saveDrawn}
        </Button>
      </div>
    </div>
  );
}
