import {
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type Ref,
  type RefObject,
} from "react";

import { Button } from "@/components/ui/button";
import { pageElementId } from "@/features/shell/format";
import type { PageSize, Rotation, Zoom } from "@/features/shell/model";
import {
  currentPageAt,
  layoutPages,
  renderScale,
  scrollTopFor,
  visibleRange,
  zoomFactor,
  type PageBox,
  type Viewport,
} from "@/features/viewer/layout";
import { errorCodeOf, type PageRenderer, type RenderJob } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, Rotation as ContractRotation } from "@/ipc/generated/contract";
import type { RasterImage } from "@/ipc/raster";

export type DocumentViewHandle = {
  /** Scrolls so that the 1-based `page` is at the top. */
  scrollToPage(page: number): void;
};

type DocumentViewProps = {
  pages: PageSize[];
  zoom: Zoom;
  rotation: Rotation;
  /** The scrolling element the pages live in. */
  scrollContainer: RefObject<HTMLElement | null>;
  /** Without a document id or renderer (demo data), pages stay placeholders. */
  doc?: DocumentId;
  renderer?: PageRenderer;
  onCurrentPageChange?: (page: number) => void;
  /** Delay before a newly mounted page asks for a render; tests pass 0. */
  requestDelayMs?: number;
  ref?: Ref<DocumentViewHandle>;
};

/** How long the render scale must stay unchanged (e.g. while resizing) before re-rendering. */
const RESCALE_DELAY_MS = 150;
/**
 * A page must stay mounted this long before it is rendered. Pages that only flash by during a
 * fast scroll never cost a render or a multi-megabyte transfer.
 */
const REQUEST_DELAY_MS = 100;

const UNMEASURED: Viewport = { top: 0, width: 0, height: 0 };

const CONTRACT_ROTATION: Record<Rotation, ContractRotation> = { 0: "none", 90: "cw90", 180: "cw180", 270: "cw270" };

function measure(element: HTMLElement | null): Viewport {
  if (!element) return { top: 0, width: 0, height: 0 };
  return {
    top: element.scrollTop,
    width: element.clientWidth,
    // Before layout (and in tests) the element has no height yet; assume the window's.
    height: element.clientHeight || window.innerHeight,
  };
}

/**
 * Virtualized pages: the content has the full document height, but only pages near the
 * viewport are mounted and rendered (docs/architecture/rendering.md).
 */
export function DocumentView({
  pages,
  zoom,
  rotation,
  scrollContainer,
  doc,
  renderer,
  onCurrentPageChange,
  requestDelayMs = REQUEST_DELAY_MS,
  ref,
}: DocumentViewProps) {
  // Measured after the first commit: measuring during render would see the container before
  // siblings such as the sidebar take their space, and the first renders would use the wrong
  // scale. No page mounts until the viewport is known. (A layout effect would run before the
  // parent's ref to the container is attached.)
  const [measured, setMeasured] = useState<Viewport | null>(null);
  const viewport = measured ?? UNMEASURED;

  useEffect(() => {
    const element = scrollContainer.current;
    if (!element) return;
    const update = () => setMeasured(measure(element));
    update();
    element.addEventListener("scroll", update, { passive: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    observer?.observe(element);
    return () => {
      element.removeEventListener("scroll", update);
      observer?.disconnect();
    };
  }, [scrollContainer]);

  const factor = zoomFactor(zoom, pages, rotation, viewport);
  const layout = useMemo(() => layoutPages(pages, rotation, factor), [pages, rotation, factor]);
  const range = measured ? visibleRange(layout, viewport) : { first: 0, last: -1 };
  const currentPage = currentPageAt(layout, viewport);

  const onPageChange = useRef(onCurrentPageChange);
  useEffect(() => {
    onPageChange.current = onCurrentPageChange;
  });
  useEffect(() => {
    onPageChange.current?.(currentPage);
  }, [currentPage]);

  useImperativeHandle(
    ref,
    () => ({
      scrollToPage(page) {
        const element = scrollContainer.current;
        if (element) element.scrollTop = scrollTopFor(layout, page);
      },
    }),
    [layout, scrollContainer],
  );

  // The first renders use the measured scale right away. After that, a new resolution is
  // rendered only once the scale settles; meanwhile the old bitmap is stretched to the new size.
  const scale = renderScale(factor, window.devicePixelRatio);
  const [settledScale, setSettledScale] = useState<number | null>(null);
  const hasViewport = measured !== null;
  useEffect(() => {
    if (!hasViewport || scale === settledScale) return;
    const delay = settledScale === null ? 0 : RESCALE_DELAY_MS;
    const timer = window.setTimeout(() => setSettledScale(scale), delay);
    return () => window.clearTimeout(timer);
  }, [hasViewport, scale, settledScale]);
  const renderAt = settledScale ?? scale;

  const slots = [];
  for (let index = range.first; index <= range.last; index++) {
    slots.push(
      <PageSlot
        key={index}
        index={index}
        box={layout.boxes[index]!}
        doc={doc}
        renderer={renderer}
        scale={renderAt}
        rotation={CONTRACT_ROTATION[rotation]}
        requestDelayMs={requestDelayMs}
      />,
    );
  }

  return (
    <div className="relative w-full" style={{ height: layout.totalHeight }}>
      {slots}
    </div>
  );
}

type SlotState = { kind: "loading" } | { kind: "ready" } | { kind: "failed"; message: string };

type PageSlotProps = {
  index: number;
  box: PageBox;
  doc?: DocumentId;
  renderer?: PageRenderer;
  scale: number;
  rotation: ContractRotation;
  requestDelayMs: number;
};

/** Draws a raster into the canvas at its native resolution; CSS scales it to the page box. */
function draw(canvas: HTMLCanvasElement | null, raster: RasterImage) {
  const context = canvas?.getContext("2d");
  if (!canvas || !context) return;
  canvas.width = raster.width;
  canvas.height = raster.height;
  context.putImageData(new ImageData(raster.pixels, raster.width, raster.height), 0, 0);
}

function PageSlot({ index, box, doc, renderer, scale, rotation, requestDelayMs }: PageSlotProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<SlotState>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!renderer || doc === undefined) return;
    let job: RenderJob | null = null;
    const start = () => {
      job = renderer.render({ doc, pageIndex: index, scale, rotation });
      job.result.then(
        (raster) => {
          draw(canvasRef.current, raster);
          setState({ kind: "ready" });
        },
        (error: unknown) => {
          const code = errorCodeOf(error);
          // Cancelled: the page left the view or the view changed. Unknown document: it was closed.
          if (code === "cancelled" || code === "unknownDocument") return;
          setState({ kind: "failed", message: strings.canvas.pageRenderFailed });
        },
      );
    };
    const timer = requestDelayMs > 0 ? window.setTimeout(start, requestDelayMs) : undefined;
    if (timer === undefined) start();
    // Leaving the rendered range, or a new scale or rotation, cancels the old request.
    return () => {
      window.clearTimeout(timer);
      (job as RenderJob | null)?.cancel();
    };
  }, [renderer, doc, index, scale, rotation, attempt, requestDelayMs]);

  return (
    <div
      id={pageElementId(index + 1)}
      role="img"
      aria-label={strings.canvas.page(index + 1)}
      data-state={state.kind}
      className="absolute left-1/2 flex -translate-x-1/2 items-center justify-center overflow-hidden bg-white text-sm text-neutral-400 shadow-sm ring-1 ring-black/10"
      style={{ top: box.top, width: box.width, height: box.height }}
    >
      <canvas ref={canvasRef} aria-hidden className="absolute inset-0 size-full" />
      {state.kind === "loading" && <span aria-hidden>{index + 1}</span>}
      {state.kind === "failed" && (
        <div role="alert" className="relative flex flex-col items-center gap-2 bg-white/90 p-4 text-neutral-700">
          <span>{state.message}</span>
          <Button
            size="sm"
            variant="outline"
            // The page is always white paper, whatever the theme.
            className="border-neutral-300 bg-white text-neutral-900 hover:bg-neutral-100 hover:text-neutral-900 dark:border-neutral-300 dark:bg-white dark:hover:bg-neutral-100"
            onClick={() => {
              setState({ kind: "loading" });
              setAttempt((count) => count + 1);
            }}
          >
            {strings.error.retry}
          </Button>
        </div>
      )}
    </div>
  );
}
