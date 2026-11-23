// Dialog for drawing a watermark on the open document (B6-06, #591).

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DEFAULT_WATERMARK_FORM,
  watermarkProblemWith,
  type WatermarkForm,
} from "@/features/batch/watermark";
import { WatermarkFields } from "@/features/batch/WatermarkFields";
import {
  editOf,
  pagesOf,
  placement,
  type PageChoice,
} from "@/features/batch/documentWatermark";
import { drawRaster, type PageRenderer } from "@/features/viewer/renderer";
import type { AnnotationsApi } from "@/features/annotations/source";
import { strings } from "@/i18n";
import type {
  DocumentId,
  Edit,
  IpcError,
  PageSize,
  StampImageInfo,
} from "@/ipc/generated/contract";

const t = strings.documentWatermark;

type DocumentWatermarkDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  pages: PageSize[];
  sensitive?: boolean;
  renderer?: PageRenderer;
  annotationsApi?: Pick<AnnotationsApi, "pickStampImage">;
  applyEdit: (edit: Edit) => Promise<void>;
  onFinished: (message: string) => void;
};

export function DocumentWatermarkDialog({
  open,
  onOpenChange,
  doc,
  pages,
  sensitive,
  renderer,
  annotationsApi,
  applyEdit,
  onFinished,
}: DocumentWatermarkDialogProps) {
  const [form, setForm] = useState<WatermarkForm>(DEFAULT_WATERMARK_FORM);
  const [pageChoice, setPageChoice] = useState<PageChoice>("all");
  const [rangeText, setRangeText] = useState("");
  const [imageInfo, setImageInfo] = useState<StampImageInfo | null>(null);
  const [previewPageIndex, setPreviewPageIndex] = useState(0);
  const [applying, setApplying] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [canvasReady, setCanvasReady] = useState(false);
  const setCanvas = useCallback((el: HTMLCanvasElement | null) => {
    canvasRef.current = el;
    setCanvasReady(el !== null);
  }, []);

  // Reset state when opening
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setForm(DEFAULT_WATERMARK_FORM);
      setPageChoice("all");
      setRangeText("");
      setImageInfo(null);
      setPreviewPageIndex(0);
      setApplying(false);
      setErrorMessage(null);
    }
  }

  const targetPages = pagesOf(pageChoice, rangeText, pages.length);
  const formProblem = watermarkProblemWith(form);
  const hasPages = targetPages !== null && targetPages.length > 0;
  const canApply =
    !applying &&
    formProblem === null &&
    hasPages &&
    (form.type !== "picture" || imageInfo !== null);

  const currentPage = useMemo(
    () => pages[previewPageIndex] ?? { widthPt: 595, heightPt: 842 },
    [pages, previewPageIndex],
  );

  // Render preview canvas
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!open || !canvas) return;

    let cancelled = false;

    // Helper to draw watermark overlay on top of canvas context
    const drawOverlay = (ctx: CanvasRenderingContext2D) => {
      ctx.save();
      const scaleX = canvas.width / currentPage.widthPt;
      const scaleY = canvas.height / currentPage.heightPt;
      ctx.scale(scaleX, scaleY);

      let markW: number;
      let markH: number;
      if (form.type === "text") {
        ctx.font = `${form.size}px sans-serif`;
        const metrics = ctx.measureText(form.text);
        markW = Math.max(metrics.width, 1);
        markH = form.size;
      } else {
        markW = form.pictureWidth;
        const ratio = imageInfo ? imageInfo.height / imageInfo.width : 0.6;
        markH = markW * ratio;
      }

      const p = placement(
        currentPage,
        { width: markW, height: markH },
        form.diagonal,
        form.place,
      );

      ctx.translate(p.center.x, p.center.y);
      ctx.rotate(-p.angle);
      ctx.scale(p.scale, p.scale);
      ctx.globalAlpha = Math.max(0.05, Math.min(1.0, form.opacity / 100));

      if (form.type === "text") {
        ctx.fillStyle = form.color;
        ctx.font = `${form.size}px sans-serif`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(form.text, 0, 0);
      } else {
        ctx.strokeStyle = "#444444";
        ctx.lineWidth = 1.5;
        ctx.strokeRect(-markW / 2, -markH / 2, markW, markH);
        ctx.fillStyle = "rgba(100, 100, 100, 0.2)";
        ctx.fillRect(-markW / 2, -markH / 2, markW, markH);
        ctx.fillStyle = "#333333";
        ctx.font = "16px sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(t.pictureLabel, 0, 0);
      }
      ctx.restore();
    };

    if (sensitive || !renderer) {
      // Sensitive document or no renderer: blank background
      canvas.width = Math.round(currentPage.widthPt * 0.5);
      canvas.height = Math.round(currentPage.heightPt * 0.5);
      const ctx = canvas.getContext("2d");
      if (ctx) {
        ctx.fillStyle = "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        ctx.strokeStyle = "#cccccc";
        ctx.lineWidth = 1;
        ctx.strokeRect(0, 0, canvas.width, canvas.height);
        drawOverlay(ctx);
      }
      return;
    }

    // Normal document: call renderer
    const job = renderer.render({
      doc,
      pageIndex: previewPageIndex,
      scale: 0.5,
      rotation: "none",
    });

    job.result
      .then((raster) => {
        if (cancelled) return;
        drawRaster(canvas, raster);
        const ctx = canvas.getContext("2d");
        if (ctx) {
          drawOverlay(ctx);
        }
      })
      .catch(() => {
        if (cancelled) return;
        const ctx = canvas.getContext("2d");
        if (ctx) {
          canvas.width = Math.round(currentPage.widthPt * 0.5);
          canvas.height = Math.round(currentPage.heightPt * 0.5);
          ctx.fillStyle = "#ffffff";
          ctx.fillRect(0, 0, canvas.width, canvas.height);
          drawOverlay(ctx);
        }
      });

    return () => {
      cancelled = true;
      job.cancel();
    };
  }, [
    open,
    canvasReady,
    doc,
    previewPageIndex,
    sensitive,
    renderer,
    currentPage,
    form,
    imageInfo,
  ]);

  const onPickPicture = () => {
    if (!annotationsApi?.pickStampImage) return;
    annotationsApi.pickStampImage(doc).then((info) => {
      if (info) {
        setImageInfo(info);
      }
    });
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canApply || !targetPages) return;

    setApplying(true);
    setErrorMessage(null);

    const edit = editOf(form, targetPages, imageInfo?.image ?? null);
    applyEdit(edit)
      .then(() => {
        setApplying(false);
        onOpenChange(false);
        onFinished(t.applied);
      })
      .catch((err: unknown) => {
        setApplying(false);
        const code = (err as Partial<IpcError> | null)?.code;
        if (code === "signed") {
          setErrorMessage(t.signedDocument);
        } else if (code === "notAllowed") {
          setErrorMessage(strings.permissions.notAllowed);
        } else if (code === "limitExceeded") {
          setErrorMessage(strings.pages.saveFirst);
        } else {
          setErrorMessage(
            (err as Partial<IpcError> | null)?.message ?? t.failed,
          );
        }
      });
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !applying && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-3xl">
        <form onSubmit={onSubmit} className="grid gap-5">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>

          <div className="grid gap-6 sm:grid-cols-2">
            {/* Left Column: Form Fields */}
            <div className="grid gap-4">
              <WatermarkFields
                form={form}
                onChange={(changes) => setForm((prev) => ({ ...prev, ...changes }))}
                disabled={applying}
                hidePages={true}
              />

              {form.type === "picture" && (
                <div className="grid gap-2 text-sm">
                  <div className="flex items-center gap-3">
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      disabled={applying}
                      onClick={onPickPicture}
                    >
                      {imageInfo ? t.changePicture : t.choosePicture}
                    </Button>
                    {imageInfo ? (
                      <span className="text-xs text-muted-foreground">
                        {t.pictureSize(imageInfo.width, imageInfo.height)}
                      </span>
                    ) : (
                      <span className="text-xs text-destructive">
                        {t.noPictureChosen}
                      </span>
                    )}
                  </div>
                </div>
              )}

              {/* Page Selection */}
              <fieldset className="grid gap-2 text-sm">
                <legend className="mb-1 font-medium">{t.pages}</legend>
                <div className="grid gap-2">
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="target-pages"
                      value="all"
                      disabled={applying}
                      checked={pageChoice === "all"}
                      onChange={() => setPageChoice("all")}
                    />
                    {t.pagesAll}
                  </label>
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="target-pages"
                      value="first"
                      disabled={applying}
                      checked={pageChoice === "first"}
                      onChange={() => setPageChoice("first")}
                    />
                    {t.pagesFirst}
                  </label>
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="target-pages"
                      value="allButFirst"
                      disabled={applying}
                      checked={pageChoice === "allButFirst"}
                      onChange={() => setPageChoice("allButFirst")}
                    />
                    {t.pagesAllButFirst}
                  </label>
                  <label className="flex items-center gap-2">
                    <input
                      type="radio"
                      name="target-pages"
                      value="range"
                      disabled={applying}
                      checked={pageChoice === "range"}
                      onChange={() => setPageChoice("range")}
                    />
                    {t.pagesRange}
                  </label>
                  {pageChoice === "range" && (
                    <div className="ml-6 grid gap-1">
                      <input
                        type="text"
                        placeholder={t.pagesRangePlaceholder}
                        disabled={applying}
                        value={rangeText}
                        onChange={(e) => setRangeText(e.target.value)}
                        className="h-8 rounded-md border bg-background px-2 text-sm"
                      />
                      {!hasPages && rangeText.trim() !== "" && (
                        <p className="text-xs text-destructive">{t.invalidPages}</p>
                      )}
                    </div>
                  )}
                </div>
              </fieldset>
            </div>

            {/* Right Column: Preview */}
            <div className="flex flex-col items-center justify-between rounded-md border bg-muted/20 p-3">
              <div className="flex w-full items-center justify-between pb-2 text-xs font-medium text-muted-foreground">
                <span>{t.preview}</span>
                {pages.length > 1 && (
                  <div className="flex items-center gap-2">
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={applying || previewPageIndex <= 0}
                      onClick={() => setPreviewPageIndex((p) => Math.max(0, p - 1))}
                    >
                      {t.prevPage}
                    </Button>
                    <span>{t.previewPage(previewPageIndex + 1, pages.length)}</span>
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={applying || previewPageIndex >= pages.length - 1}
                      onClick={() =>
                        setPreviewPageIndex((p) => Math.min(pages.length - 1, p + 1))
                      }
                    >
                      {t.nextPage}
                    </Button>
                  </div>
                )}
              </div>

              <div className="flex max-h-[360px] max-w-[280px] items-center justify-center overflow-hidden rounded border bg-background shadow-sm">
                <canvas
                  ref={setCanvas}
                  aria-label={t.preview}
                  className="max-h-[360px] max-w-[280px] object-contain"
                />
              </div>

              <p className="mt-2 text-center text-[11px] text-muted-foreground">
                {t.previewFontNote}
              </p>
            </div>
          </div>

          {/* Error Message */}
          {errorMessage && (
            <p className="text-sm font-medium text-destructive">{errorMessage}</p>
          )}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={applying}
              onClick={() => onOpenChange(false)}
            >
              {t.cancel}
            </Button>
            <Button type="submit" disabled={!canApply}>
              {t.apply}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
