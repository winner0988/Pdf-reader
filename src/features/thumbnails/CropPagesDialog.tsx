// Dialog for cropping pages (B6-04, #589).

import { useEffect, useId, useMemo, useRef, useState, type FormEvent, type PointerEvent } from "react";
import { FileLock2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CSS_PX_PER_PT, renderScale } from "@/features/viewer/layout";
import { drawRaster, type PageRenderer, type RenderJob } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { CropMargins, DocumentId, PageSize } from "@/ipc/generated/contract";
import {
  cropsNothing,
  firstTooSmall,
  marginsFromDrag,
  pagesFor,
  type CropChoice,
} from "./crop";

const t = strings.pages.crop;

export type CropPagesDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc?: DocumentId;
  renderer?: PageRenderer;
  sensitive?: boolean;
  pages: readonly PageSize[];
  selectedPages: readonly number[];
  currentPage: number;
  onCrop: (pages: number[], margins: CropMargins) => void;
};

export function CropPagesDialog({
  open,
  onOpenChange,
  doc,
  renderer,
  sensitive = false,
  pages,
  selectedPages,
  currentPage,
  onCrop,
}: CropPagesDialogProps) {
  const [choice, setChoice] = useState<CropChoice>("selected");
  const [margins, setMargins] = useState<CropMargins>({ top: 0, right: 0, bottom: 0, left: 0 });
  const [wasOpen, setWasOpen] = useState(open);

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const previewBoxRef = useRef<HTMLDivElement | null>(null);
  const dragStartRef = useRef<{ x: number; y: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  const topId = useId();
  const bottomId = useId();
  const leftId = useId();
  const rightId = useId();
  const errorId = useId();

  // Reset state whenever opened
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setChoice(selectedPages.length > 0 ? "selected" : "current");
      setMargins({ top: 0, right: 0, bottom: 0, left: 0 });
      dragStartRef.current = null;
      setDragging(false);
    }
  }

  const pageCount = pages.length;
  const current0 = Math.max(0, Math.min(currentPage - 1, pageCount - 1));

  const chosenPages = useMemo(
    () => pagesFor(choice, pageCount, selectedPages, current0),
    [choice, pageCount, selectedPages, current0],
  );

  const previewPageIndex = chosenPages[0] ?? current0;
  const pageSize = pages[previewPageIndex] ?? { widthPt: 612, heightPt: 792 };

  // Preview dimensions inside max 320x360 box
  const maxBoxW = 320;
  const maxBoxH = 360;
  const pageAspect = pageSize.widthPt / pageSize.heightPt;
  let previewW = maxBoxW;
  let previewH = Math.round(maxBoxW / pageAspect);
  if (previewH > maxBoxH) {
    previewH = maxBoxH;
    previewW = Math.round(maxBoxH * pageAspect);
  }
  const previewScalePt = previewW / pageSize.widthPt; // CSS px per PDF point in preview

  // Render the preview page
  useEffect(() => {
    if (!open || sensitive || !renderer || doc === undefined || pageCount === 0) {
      return;
    }
    const factor = previewW / (pageSize.widthPt * CSS_PX_PER_PT);
    const scale = renderScale(factor, window.devicePixelRatio || 1);
    let job: RenderJob | null = null;

    job = renderer.render({ doc, pageIndex: previewPageIndex, scale, rotation: "none" });
    job.result.then(
      (raster) => {
        if (canvasRef.current) {
          drawRaster(canvasRef.current, raster);
        }
      },
      () => {},
    );

    return () => {
      job?.cancel();
    };
  }, [open, sensitive, renderer, doc, previewPageIndex, pageSize.widthPt, previewW, pageCount]);

  // Validation
  const tooSmallIndex = useMemo(
    () => firstTooSmall(chosenPages, pages, margins),
    [chosenPages, pages, margins],
  );
  const nothingCropped = cropsNothing(margins);
  const canConfirm = !nothingCropped && tooSmallIndex === null && chosenPages.length > 0;

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (sensitive || !previewBoxRef.current) return;
    const rect = previewBoxRef.current.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    dragStartRef.current = { x, y };
    setDragging(true);
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!dragging || !dragStartRef.current || !previewBoxRef.current) return;
    const rect = previewBoxRef.current.getBoundingClientRect();
    const currentX = e.clientX - rect.left;
    const currentY = e.clientY - rect.top;
    const newMargins = marginsFromDrag(dragStartRef.current, { x: currentX, y: currentY }, pageSize, previewScalePt);
    setMargins(newMargins);
  };

  const onPointerUp = (e: PointerEvent<HTMLDivElement>) => {
    if (dragging) {
      setDragging(false);
      dragStartRef.current = null;
      try {
        e.currentTarget.releasePointerCapture(e.pointerId);
      } catch {
        // Ignored
      }
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!canConfirm) return;
    onOpenChange(false);
    onCrop(chosenPages, margins);
  };

  // Overlay positions
  const cropLeftPx = Math.max(0, margins.left * previewScalePt);
  const cropTopPx = Math.max(0, margins.top * previewScalePt);
  const cropRightPx = Math.max(0, (pageSize.widthPt - margins.right) * previewScalePt);
  const cropBottomPx = Math.max(0, (pageSize.heightPt - margins.bottom) * previewScalePt);
  const cropWidthPx = Math.max(0, cropRightPx - cropLeftPx);
  const cropHeightPx = Math.max(0, cropBottomPx - cropTopPx);

  const oddCount = useMemo(() => pagesFor("odd", pageCount, [], 0).length, [pageCount]);
  const evenCount = useMemo(() => pagesFor("even", pageCount, [], 0).length, [pageCount]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <form noValidate onSubmit={submit} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription className="sr-only">{t.title}</DialogDescription>
          </DialogHeader>

          {/* Notice always visible at top */}
          <div
            role="note"
            className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-xs leading-relaxed text-amber-900 dark:text-amber-200"
          >
            {t.notice}
          </div>

          <div className="grid grid-cols-1 gap-6 sm:grid-cols-2">
            {/* Left column: options and margin inputs */}
            <div className="flex flex-col gap-4 text-sm">
              <fieldset className="grid gap-1.5">
                <legend className="font-medium text-xs text-muted-foreground">{t.scope}</legend>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cropChoice"
                    value="selected"
                    checked={choice === "selected"}
                    disabled={selectedPages.length === 0}
                    onChange={() => setChoice("selected")}
                  />
                  <span>{t.selected(selectedPages.length)}</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cropChoice"
                    value="current"
                    checked={choice === "current"}
                    onChange={() => setChoice("current")}
                  />
                  <span>{t.current(currentPage)}</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cropChoice"
                    value="all"
                    checked={choice === "all"}
                    onChange={() => setChoice("all")}
                  />
                  <span>{t.all(pageCount)}</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cropChoice"
                    value="odd"
                    checked={choice === "odd"}
                    disabled={pageCount <= 1}
                    onChange={() => setChoice("odd")}
                  />
                  <span>{t.odd(oddCount)}</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer">
                  <input
                    type="radio"
                    name="cropChoice"
                    value="even"
                    checked={choice === "even"}
                    disabled={pageCount <= 1}
                    onChange={() => setChoice("even")}
                  />
                  <span>{t.even(evenCount)}</span>
                </label>
              </fieldset>

              <div className="grid gap-2">
                <span className="font-medium text-xs text-muted-foreground">{t.margins}</span>
                <div className="grid grid-cols-2 gap-2">
                  <div className="flex flex-col gap-1">
                    <label htmlFor={topId} className="text-xs font-medium">
                      {t.top}
                    </label>
                    <input
                      id={topId}
                      type="number"
                      inputMode="numeric"
                      min={0}
                      value={margins.top}
                      onChange={(e) => {
                        const val = Math.max(0, Math.round(Number(e.target.value) || 0));
                        setMargins((prev) => ({ ...prev, top: val }));
                      }}
                      className="h-8 rounded-md border bg-background px-2 text-sm"
                    />
                  </div>
                  <div className="flex flex-col gap-1">
                    <label htmlFor={bottomId} className="text-xs font-medium">
                      {t.bottom}
                    </label>
                    <input
                      id={bottomId}
                      type="number"
                      inputMode="numeric"
                      min={0}
                      value={margins.bottom}
                      onChange={(e) => {
                        const val = Math.max(0, Math.round(Number(e.target.value) || 0));
                        setMargins((prev) => ({ ...prev, bottom: val }));
                      }}
                      className="h-8 rounded-md border bg-background px-2 text-sm"
                    />
                  </div>
                  <div className="flex flex-col gap-1">
                    <label htmlFor={leftId} className="text-xs font-medium">
                      {t.left}
                    </label>
                    <input
                      id={leftId}
                      type="number"
                      inputMode="numeric"
                      min={0}
                      value={margins.left}
                      onChange={(e) => {
                        const val = Math.max(0, Math.round(Number(e.target.value) || 0));
                        setMargins((prev) => ({ ...prev, left: val }));
                      }}
                      className="h-8 rounded-md border bg-background px-2 text-sm"
                    />
                  </div>
                  <div className="flex flex-col gap-1">
                    <label htmlFor={rightId} className="text-xs font-medium">
                      {t.right}
                    </label>
                    <input
                      id={rightId}
                      type="number"
                      inputMode="numeric"
                      min={0}
                      value={margins.right}
                      onChange={(e) => {
                        const val = Math.max(0, Math.round(Number(e.target.value) || 0));
                        setMargins((prev) => ({ ...prev, right: val }));
                      }}
                      className="h-8 rounded-md border bg-background px-2 text-sm"
                    />
                  </div>
                </div>
                <p className="text-xs text-muted-foreground">{t.unitNote}</p>
              </div>
            </div>

            {/* Right column: preview */}
            <div className="flex flex-col items-center justify-center">
              <div
                ref={previewBoxRef}
                role="region"
                aria-label={t.previewAria}
                onPointerDown={onPointerDown}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onPointerCancel={onPointerUp}
                className="relative overflow-hidden rounded-md border bg-muted/40 shadow-inner select-none cursor-crosshair touch-none"
                style={{ width: previewW, height: previewH }}
              >
                {sensitive ? (
                  <div className="flex size-full items-center justify-center text-muted-foreground">
                    <FileLock2 className="size-10" />
                  </div>
                ) : (
                  <canvas ref={canvasRef} aria-hidden className="size-full" />
                )}

                {/* Crop area overlay mask */}
                <div
                  aria-hidden
                  className="pointer-events-none absolute border-2 border-primary"
                  style={{
                    left: cropLeftPx,
                    top: cropTopPx,
                    width: cropWidthPx,
                    height: cropHeightPx,
                    boxShadow: "0 0 0 9999px rgba(0, 0, 0, 0.45)",
                  }}
                />
              </div>
            </div>
          </div>

          {/* Validation warning */}
          {tooSmallIndex !== null && (
            <p id={errorId} role="alert" className="text-xs text-destructive">
              {t.tooSmall(tooSmallIndex + 1)}
            </p>
          )}
          {tooSmallIndex === null && nothingCropped && (
            <p id={errorId} role="alert" className="text-xs text-muted-foreground">
              {t.cropsNothing}
            </p>
          )}

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              {t.cancel}
            </Button>
            <Button type="submit" disabled={!canConfirm}>
              {t.confirm}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
