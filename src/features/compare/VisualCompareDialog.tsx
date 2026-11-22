import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Layers,
  Loader2,
  SquareSplitHorizontal,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { CompareTabOption } from "@/features/compare/CompareDialog";
import {
  compareScale,
  diffRegions,
  pagePairs,
  type PagePair,
  type Rect,
} from "@/features/compare/visual";
import {
  createPageRenderer,
  drawRaster,
  tauriRenderApi,
  type RenderApi,
} from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, PageSize } from "@/ipc/generated/contract";
import type { RasterImage } from "@/ipc/raster";
import { cn } from "@/lib/utils";

const t = strings.compare.visual;

export type DisplayMode = "sideBySide" | "overlay";

export type CachedPageDiff = {
  regions: Rect[];
  truncated: boolean;
  uncomparable: boolean;
  sizeDiff: boolean;
  onlyIn?: string;
};

export type VisualCompareDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentDoc: DocumentId;
  currentDisplayName?: string;
  currentPages?: PageSize[];
  otherTabs: CompareTabOption[];
  renderApi?: RenderApi;
  onJumpToPage?: (page: number) => void;
};

export function VisualCompareDialog({
  open,
  onOpenChange,
  currentDoc,
  currentDisplayName = "Document A",
  currentPages = [],
  otherTabs,
  renderApi = tauriRenderApi,
  onJumpToPage,
}: VisualCompareDialogProps) {
  const [selectedDoc, setSelectedDoc] = useState<DocumentId>(() => otherTabs[0]?.doc ?? 0);
  const [started, setStarted] = useState(false);
  const [pageIndex, setPageIndex] = useState(0);
  const [displayMode, setDisplayMode] = useState<DisplayMode>("sideBySide");
  const [selectedDiffIndex, setSelectedDiffIndex] = useState(0);
  const [scanning, setScanning] = useState(false);
  const [loadingPage, setLoadingPage] = useState(false);

  // Cached difference regions per page index
  const [diffCache, setDiffCache] = useState<Map<number, CachedPageDiff>>(() => new Map());

  // Active raster images for the currently viewed page
  const [rasterA, setRasterA] = useState<RasterImage | null>(null);
  const [rasterB, setRasterB] = useState<RasterImage | null>(null);
  const [currentDiff, setCurrentDiff] = useState<CachedPageDiff | null>(null);

  const renderer = useMemo(() => createPageRenderer(renderApi), [renderApi]);

  const canvasARef = useRef<HTMLCanvasElement | null>(null);
  const canvasBRef = useRef<HTMLCanvasElement | null>(null);
  const canvasOverlayRef = useRef<HTMLCanvasElement | null>(null);
  const selectedBoxRef = useRef<HTMLDivElement | null>(null);

  const activeJobsRef = useRef<{ cancelA?: () => void; cancelB?: () => void }>({});
  const scanAbortRef = useRef(false);

  const selectedTab = otherTabs.find((t) => t.doc === selectedDoc);
  const targetDisplayName = selectedTab?.displayName ?? "Document B";
  const targetPages = useMemo(() => selectedTab?.pages ?? [], [selectedTab?.pages]);

  const pairs: PagePair[] = useMemo(
    () => pagePairs(currentPages.length, targetPages.length),
    [currentPages.length, targetPages.length],
  );
  const totalPages = Math.max(1, pairs.length);

  // Cancel any pending render jobs
  const cancelActiveJobs = useCallback(() => {
    activeJobsRef.current.cancelA?.();
    activeJobsRef.current.cancelB?.();
    activeJobsRef.current = {};
  }, []);

  const handleClose = () => {
    cancelActiveJobs();
    scanAbortRef.current = true;
    setStarted(false);
    setPageIndex(0);
    setSelectedDiffIndex(0);
    setDiffCache(new Map());
    setRasterA(null);
    setRasterB(null);
    setCurrentDiff(null);
    setScanning(false);
    setLoadingPage(false);
    onOpenChange(false);
  };

  // Helper to render and compute diff for a given page index
  const computePageDiff = useCallback(
    async (
      pIndex: number,
    ): Promise<{
      diff: CachedPageDiff;
      rA: RasterImage | null;
      rB: RasterImage | null;
    }> => {
      const pair = pairs[pIndex] ?? { a: null, b: null };

      // Case 1: Page only in Doc A
      if (pair.a !== null && pair.b === null) {
        const sizeA = currentPages[pair.a];
        if (!sizeA) {
          return {
            diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
            rA: null,
            rB: null,
          };
        }
        const scaleA = compareScale(sizeA);
        const expectedWA = Math.round(sizeA.widthPt * scaleA);
        const jobA = renderer.render({
          doc: currentDoc,
          pageIndex: pair.a,
          scale: scaleA,
          rotation: "none",
        });
        activeJobsRef.current.cancelA = () => jobA.cancel();
        try {
          const rA = await jobA.result;
          if (Math.abs(rA.width - expectedWA) > 2) {
            return {
              diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
              rA: null,
              rB: null,
            };
          }
          return {
            diff: {
              regions: [],
              truncated: false,
              uncomparable: false,
              sizeDiff: false,
              onlyIn: currentDisplayName,
            },
            rA,
            rB: null,
          };
        } catch {
          return {
            diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
            rA: null,
            rB: null,
          };
        }
      }

      // Case 2: Page only in Doc B
      if (pair.a === null && pair.b !== null) {
        const sizeB = targetPages[pair.b];
        if (!sizeB) {
          return {
            diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
            rA: null,
            rB: null,
          };
        }
        const scaleB = compareScale(sizeB);
        const expectedWB = Math.round(sizeB.widthPt * scaleB);
        const jobB = renderer.render({
          doc: selectedDoc,
          pageIndex: pair.b,
          scale: scaleB,
          rotation: "none",
        });
        activeJobsRef.current.cancelB = () => jobB.cancel();
        try {
          const rB = await jobB.result;
          if (Math.abs(rB.width - expectedWB) > 2) {
            return {
              diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
              rA: null,
              rB: null,
            };
          }
          return {
            diff: {
              regions: [],
              truncated: false,
              uncomparable: false,
              sizeDiff: false,
              onlyIn: targetDisplayName,
            },
            rA: null,
            rB,
          };
        } catch {
          return {
            diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
            rA: null,
            rB: null,
          };
        }
      }

      // Case 3: Page in both Doc A and Doc B
      if (pair.a !== null && pair.b !== null) {
        const sizeA = currentPages[pair.a];
        const sizeB = targetPages[pair.b];
        if (!sizeA || !sizeB) {
          return {
            diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
            rA: null,
            rB: null,
          };
        }

        const sizeDiff =
          Math.abs(sizeA.widthPt - sizeB.widthPt) > 0.01 ||
          Math.abs(sizeA.heightPt - sizeB.heightPt) > 0.01;

        const scaleA = compareScale(sizeA);
        const scaleB = compareScale(sizeB);
        const expectedWA = Math.round(sizeA.widthPt * scaleA);
        const expectedWB = Math.round(sizeB.widthPt * scaleB);

        const jobA = renderer.render({
          doc: currentDoc,
          pageIndex: pair.a,
          scale: scaleA,
          rotation: "none",
        });
        const jobB = renderer.render({
          doc: selectedDoc,
          pageIndex: pair.b,
          scale: scaleB,
          rotation: "none",
        });
        activeJobsRef.current.cancelA = () => jobA.cancel();
        activeJobsRef.current.cancelB = () => jobB.cancel();

        try {
          const [rA, rB] = await Promise.all([jobA.result, jobB.result]);
          if (
            Math.abs(rA.width - expectedWA) > 2 ||
            Math.abs(rB.width - expectedWB) > 2
          ) {
            return {
              diff: { regions: [], truncated: false, uncomparable: true, sizeDiff },
              rA: null,
              rB: null,
            };
          }

          const outcome = diffRegions(rA, rB);
          return {
            diff: {
              regions: outcome.regions,
              truncated: outcome.truncated,
              uncomparable: false,
              sizeDiff,
            },
            rA,
            rB,
          };
        } catch {
          return {
            diff: { regions: [], truncated: false, uncomparable: true, sizeDiff },
            rA: null,
            rB: null,
          };
        }
      }

      return {
        diff: { regions: [], truncated: false, uncomparable: true, sizeDiff: false },
        rA: null,
        rB: null,
      };
    },
    [pairs, currentPages, targetPages, currentDoc, selectedDoc, currentDisplayName, targetDisplayName, renderer],
  );

  // Load and display a specific page
  const loadPage = useCallback(
    async (targetPage: number) => {
      cancelActiveJobs();
      setLoadingPage(true);
      setSelectedDiffIndex(0);

      try {
        const { diff, rA, rB } = await computePageDiff(targetPage);
        setDiffCache((prev) => new Map(prev).set(targetPage, diff));
        setCurrentDiff(diff);
        setRasterA(rA);
        setRasterB(rB);
      } finally {
        setLoadingPage(false);
      }
    },
    [cancelActiveJobs, computePageDiff],
  );

  // Redraw canvases whenever rasters or displayMode changes
  useEffect(() => {
    if (displayMode === "sideBySide") {
      if (canvasARef.current && rasterA) {
        drawRaster(canvasARef.current, rasterA);
      }
      if (canvasBRef.current && rasterB) {
        drawRaster(canvasBRef.current, rasterB);
      }
    } else if (displayMode === "overlay" && canvasOverlayRef.current) {
      const canvas = canvasOverlayRef.current;
      const ctx = canvas.getContext("2d");
      if (ctx) {
        if (rasterA && rasterB) {
          canvas.width = Math.max(rasterA.width, rasterB.width);
          canvas.height = Math.max(rasterA.height, rasterB.height);
          ctx.clearRect(0, 0, canvas.width, canvas.height);

          // Draw Doc A at full opacity
          const tempA = document.createElement("canvas");
          tempA.width = rasterA.width;
          tempA.height = rasterA.height;
          tempA.getContext("2d")?.putImageData(new ImageData(rasterA.pixels, rasterA.width, rasterA.height), 0, 0);

          // Draw Doc B
          const tempB = document.createElement("canvas");
          tempB.width = rasterB.width;
          tempB.height = rasterB.height;
          tempB.getContext("2d")?.putImageData(new ImageData(rasterB.pixels, rasterB.width, rasterB.height), 0, 0);

          ctx.globalAlpha = 1.0;
          ctx.drawImage(tempA, 0, 0);
          ctx.globalAlpha = 0.5;
          ctx.drawImage(tempB, 0, 0);
          ctx.globalAlpha = 1.0;
        } else if (rasterA) {
          drawRaster(canvas, rasterA);
        } else if (rasterB) {
          drawRaster(canvas, rasterB);
        }
      }
    }
  }, [rasterA, rasterB, displayMode]);

  // Scroll active difference region into view
  useEffect(() => {
    selectedBoxRef.current?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [selectedDiffIndex, pageIndex]);

  const handleStartCompare = () => {
    setDiffCache(new Map());
    setStarted(true);
    setPageIndex(0);
    setSelectedDiffIndex(0);
    void loadPage(0);
    onJumpToPage?.(1);
  };

  const handlePrevPage = () => {
    if (pageIndex > 0 && !scanning) {
      const nextP = pageIndex - 1;
      setPageIndex(nextP);
      void loadPage(nextP);
      onJumpToPage?.(nextP + 1);
    }
  };

  const handleNextPage = () => {
    if (pageIndex < totalPages - 1 && !scanning) {
      const nextP = pageIndex + 1;
      setPageIndex(nextP);
      void loadPage(nextP);
      onJumpToPage?.(nextP + 1);
    }
  };

  // Difference jumping: Next Difference
  const handleNextDiff = async () => {
    if (scanning) return;
    const currentRegions = currentDiff?.regions ?? [];

    // If more differences on the current page
    if (selectedDiffIndex + 1 < currentRegions.length) {
      setSelectedDiffIndex((i) => i + 1);
      return;
    }

    // Otherwise scan subsequent pages
    if (pageIndex >= totalPages - 1) return;

    setScanning(true);
    scanAbortRef.current = false;

    let foundPage = -1;
    let foundDiff: CachedPageDiff | null = null;
    let foundRA: RasterImage | null = null;
    let foundRB: RasterImage | null = null;

    try {
      for (let p = pageIndex + 1; p < totalPages; p++) {
        if (scanAbortRef.current) break;

        const cached = diffCache.get(p);
        if (cached) {
          if (cached.regions.length > 0) {
            foundPage = p;
            foundDiff = cached;
            break;
          }
        } else {
          const outcome = await computePageDiff(p);
          if (scanAbortRef.current) break;
          setDiffCache((prev) => new Map(prev).set(p, outcome.diff));
          if (outcome.diff.regions.length > 0) {
            foundPage = p;
            foundDiff = outcome.diff;
            foundRA = outcome.rA;
            foundRB = outcome.rB;
            break;
          }
        }
      }
    } finally {
      setScanning(false);
    }

    if (foundPage !== -1 && foundDiff) {
      setPageIndex(foundPage);
      setSelectedDiffIndex(0);
      setCurrentDiff(foundDiff);
      if (foundRA || foundRB) {
        setRasterA(foundRA);
        setRasterB(foundRB);
      } else {
        void loadPage(foundPage);
      }
      onJumpToPage?.(foundPage + 1);
    }
  };

  // Difference jumping: Previous Difference
  const handlePrevDiff = async () => {
    if (scanning) return;

    // If earlier differences on current page
    if (selectedDiffIndex > 0) {
      setSelectedDiffIndex((i) => i - 1);
      return;
    }

    // Otherwise scan backwards
    if (pageIndex <= 0) return;

    setScanning(true);
    scanAbortRef.current = false;

    let foundPage = -1;
    let foundDiff: CachedPageDiff | null = null;
    let foundRA: RasterImage | null = null;
    let foundRB: RasterImage | null = null;

    try {
      for (let p = pageIndex - 1; p >= 0; p--) {
        if (scanAbortRef.current) break;

        const cached = diffCache.get(p);
        if (cached) {
          if (cached.regions.length > 0) {
            foundPage = p;
            foundDiff = cached;
            break;
          }
        } else {
          const outcome = await computePageDiff(p);
          if (scanAbortRef.current) break;
          setDiffCache((prev) => new Map(prev).set(p, outcome.diff));
          if (outcome.diff.regions.length > 0) {
            foundPage = p;
            foundDiff = outcome.diff;
            foundRA = outcome.rA;
            foundRB = outcome.rB;
            break;
          }
        }
      }
    } finally {
      setScanning(false);
    }

    if (foundPage !== -1 && foundDiff) {
      setPageIndex(foundPage);
      setSelectedDiffIndex(foundDiff.regions.length - 1);
      setCurrentDiff(foundDiff);
      if (foundRA || foundRB) {
        setRasterA(foundRA);
        setRasterB(foundRB);
      } else {
        void loadPage(foundPage);
      }
      onJumpToPage?.(foundPage + 1);
    }
  };

  const handleStopScanning = () => {
    scanAbortRef.current = true;
    cancelActiveJobs();
    setScanning(false);
  };

  const regions = currentDiff?.regions ?? [];
  const activeWidth = Math.max(rasterA?.width ?? 0, rasterB?.width ?? 0, 1);
  const activeHeight = Math.max(rasterA?.height ?? 0, rasterB?.height ?? 0, 1);

  // Status message rendering
  const renderStatus = () => {
    if (!started || loadingPage) return null;

    if (currentDiff?.uncomparable) {
      return (
        <div role="status" className="text-sm font-medium text-destructive">
          {t.cannotCompare}
        </div>
      );
    }

    if (currentDiff?.onlyIn) {
      return (
        <div role="status" className="text-sm font-medium text-muted-foreground">
          {t.onlyInDoc(currentDiff.onlyIn)}
        </div>
      );
    }

    return (
      <div role="status" className="text-sm font-medium text-muted-foreground flex flex-wrap items-center gap-2">
        {currentDiff?.sizeDiff && <span>{t.sizeDiff}</span>}
        {currentDiff?.truncated && <span>{t.tooManyDiffs}</span>}
        {regions.length === 0 ? (
          <span>{t.noDiff}</span>
        ) : (
          <span>{t.diffInfo(selectedDiffIndex + 1, regions.length)}</span>
        )}
      </div>
    );
  };

  const renderRegionsOverlay = (rasterW: number, rasterH: number) => {
    return (
      <div className="absolute inset-0 pointer-events-none">
        {regions.map((rect, idx) => {
          const isSelected = idx === selectedDiffIndex;
          return (
            <div
              key={idx}
              ref={isSelected ? selectedBoxRef : undefined}
              aria-hidden="true"
              className={cn(
                "absolute border transition-all",
                isSelected
                  ? "border-[3px] border-destructive bg-destructive/20 ring-2 ring-primary"
                  : "border-[1.5px] border-destructive bg-destructive/10",
              )}
              style={{
                left: `${(rect.x / rasterW) * 100}%`,
                top: `${(rect.y / rasterH) * 100}%`,
                width: `${(rect.width / rasterW) * 100}%`,
                height: `${(rect.height / rasterH) * 100}%`,
              }}
            />
          );
        })}
      </div>
    );
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent showCloseButton={false} className="sm:max-w-6xl h-[90vh] flex flex-col p-4 gap-3">
        <DialogHeader className="shrink-0">
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>

        {/* Top Control Bar: Target Tab and Start */}
        <div className="flex flex-wrap items-center justify-between gap-3 border-b pb-3 shrink-0">
          <div className="flex items-center gap-2 flex-1 min-w-[280px]">
            <label htmlFor="visual-compare-target-tab" className="text-sm font-medium shrink-0">
              {t.targetTab}
            </label>
            <select
              id="visual-compare-target-tab"
              value={selectedDoc}
              onChange={(e) => {
                cancelActiveJobs();
                scanAbortRef.current = true;
                setSelectedDoc(Number(e.target.value));
                setStarted(false);
                setPageIndex(0);
                setSelectedDiffIndex(0);
                setDiffCache(new Map());
                setRasterA(null);
                setRasterB(null);
                setCurrentDiff(null);
                setScanning(false);
                setLoadingPage(false);
              }}
              disabled={scanning || loadingPage}
              aria-label={t.chooseTab}
              className="flex-1 h-9 rounded-md border border-input bg-background px-3 py-1 text-sm shadow-xs focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary disabled:opacity-50"
            >
              {otherTabs.map((tab) => (
                <option key={tab.tab} value={tab.doc}>
                  {tab.displayName}
                </option>
              ))}
            </select>
            <Button
              type="button"
              onClick={handleStartCompare}
              disabled={scanning || loadingPage || selectedDoc === undefined}
              className="shrink-0 text-white dark:text-primary-foreground"
            >
              {t.start}
            </Button>
          </div>

          <Button type="button" variant="outline" onClick={handleClose} className="shrink-0">
            {t.close}
          </Button>
        </div>

        {/* Secondary Control Bar: Visible once comparison is started */}
        {started && (
          <div className="flex flex-wrap items-center justify-between gap-3 border-b pb-2 shrink-0">
            {/* Page navigation */}
            <div className="flex items-center gap-1.5">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handlePrevPage}
                disabled={pageIndex <= 0 || scanning || loadingPage}
                aria-label={t.prevPage}
              >
                <ChevronLeft className="size-4 mr-1" />
                {t.prevPage}
              </Button>
              <span className="text-sm px-2 font-medium">
                {t.pageInfo(pageIndex + 1, totalPages)}
              </span>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handleNextPage}
                disabled={pageIndex >= totalPages - 1 || scanning || loadingPage}
                aria-label={t.nextPage}
              >
                {t.nextPage}
                <ChevronRight className="size-4 ml-1" />
              </Button>
            </div>

            {/* Display mode buttons */}
            <div className="flex items-center gap-1 bg-muted p-0.5 rounded-lg border">
              <Button
                type="button"
                variant={displayMode === "sideBySide" ? "secondary" : "ghost"}
                size="sm"
                onClick={() => setDisplayMode("sideBySide")}
                className="h-8 gap-1.5"
              >
                <SquareSplitHorizontal className="size-4" />
                {t.sideBySide}
              </Button>
              <Button
                type="button"
                variant={displayMode === "overlay" ? "secondary" : "ghost"}
                size="sm"
                onClick={() => setDisplayMode("overlay")}
                className="h-8 gap-1.5"
              >
                <Layers className="size-4" />
                {t.overlay}
              </Button>
            </div>

            {/* Difference navigation */}
            <div className="flex items-center gap-1.5">
              {scanning ? (
                <div className="flex items-center gap-2">
                  <span className="text-sm text-muted-foreground flex items-center gap-1">
                    <Loader2 className="size-4 animate-spin" />
                    {t.comparing}
                  </span>
                  <Button
                    type="button"
                    variant="destructive"
                    size="sm"
                    onClick={handleStopScanning}
                  >
                    {t.stop}
                  </Button>
                </div>
              ) : (
                <>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handlePrevDiff}
                    disabled={
                      loadingPage ||
                      (pageIndex <= 0 && selectedDiffIndex <= 0)
                    }
                    aria-label={t.prevDiff}
                  >
                    <ChevronLeft className="size-4 mr-1" />
                    {t.prevDiff}
                  </Button>
                  {regions.length > 0 && (
                    <span className="text-sm px-1.5 font-medium">
                      {t.diffInfo(selectedDiffIndex + 1, regions.length)}
                    </span>
                  )}
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleNextDiff}
                    disabled={
                      loadingPage ||
                      (pageIndex >= totalPages - 1 && selectedDiffIndex >= regions.length - 1)
                    }
                    aria-label={t.nextDiff}
                  >
                    {t.nextDiff}
                    <ChevronRight className="size-4 ml-1" />
                  </Button>
                </>
              )}
            </div>
          </div>
        )}

        {/* Status display */}
        {renderStatus()}

        {/* Visual comparison canvas viewport */}
        <div
          tabIndex={0}
          role="region"
          aria-label={t.title}
          className="flex-1 min-h-0 overflow-auto bg-muted/30 rounded-md border p-4 flex items-center justify-center focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary"
        >
          {!started ? (
            <div className="text-center text-sm text-muted-foreground py-12">
              {t.chooseTab}
            </div>
          ) : loadingPage ? (
            <div className="flex flex-col items-center gap-2 text-sm text-muted-foreground py-12">
              <Loader2 className="size-6 animate-spin text-primary" />
              <span>{t.comparing}</span>
            </div>
          ) : currentDiff?.uncomparable ? (
            <div className="text-center text-sm text-destructive py-12">
              {t.cannotCompare}
            </div>
          ) : displayMode === "sideBySide" ? (
            <div className="flex flex-col md:flex-row gap-4 items-start justify-center w-full">
              {/* Document A side */}
              <div className="flex-1 flex flex-col items-center min-w-0">
                <span className="text-xs font-semibold text-muted-foreground mb-1 truncate max-w-full">
                  {currentDisplayName}
                </span>
                {rasterA ? (
                  <div
                    role="img"
                    aria-label={t.imageAriaLabel(currentDisplayName, pageIndex + 1)}
                    className="relative border shadow-xs bg-white rounded-xs overflow-hidden max-w-full"
                  >
                    <canvas ref={canvasARef} className="block max-w-full h-auto" />
                    {renderRegionsOverlay(rasterA.width, rasterA.height)}
                  </div>
                ) : (
                  <div className="text-xs text-muted-foreground italic p-8 border rounded-md">
                    {t.onlyInDoc(targetDisplayName)}
                  </div>
                )}
              </div>

              {/* Document B side */}
              <div className="flex-1 flex flex-col items-center min-w-0">
                <span className="text-xs font-semibold text-muted-foreground mb-1 truncate max-w-full">
                  {targetDisplayName}
                </span>
                {rasterB ? (
                  <div
                    role="img"
                    aria-label={t.imageAriaLabel(targetDisplayName, pageIndex + 1)}
                    className="relative border shadow-xs bg-white rounded-xs overflow-hidden max-w-full"
                  >
                    <canvas ref={canvasBRef} className="block max-w-full h-auto" />
                    {renderRegionsOverlay(rasterB.width, rasterB.height)}
                  </div>
                ) : (
                  <div className="text-xs text-muted-foreground italic p-8 border rounded-md">
                    {t.onlyInDoc(currentDisplayName)}
                  </div>
                )}
              </div>
            </div>
          ) : (
            /* Overlay mode */
            <div className="flex flex-col items-center min-w-0">
              <span className="text-xs font-semibold text-muted-foreground mb-1">
                {currentDisplayName} + {targetDisplayName}
              </span>
              <div
                role="img"
                aria-label={t.imageAriaLabel(`${currentDisplayName} / ${targetDisplayName}`, pageIndex + 1)}
                className="relative border shadow-xs bg-white rounded-xs overflow-hidden max-w-full"
              >
                <canvas ref={canvasOverlayRef} className="block max-w-full h-auto" />
                {renderRegionsOverlay(activeWidth, activeHeight)}
              </div>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
