import {
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
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
  anchorAt,
  contentWidth,
  currentPageAt,
  layoutPages,
  pageLeft,
  rectToBox,
  renderScale,
  rotateAnchor,
  scrollForAnchor,
  scrollTopFor,
  visibleRange,
  zoomFactor,
  type Layout,
  type PageBox,
  type Viewport,
  type ViewportPoint,
} from "@/features/viewer/layout";
import { linkHoverText } from "@/features/links/text";
import type { LinkSource } from "@/features/links/source";
import { pageOverlay, revealScroll, type Highlight, type Highlights, type PageOverlay } from "@/features/viewer/highlights";
import { errorCodeOf, type PageRenderer, type RenderJob } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, PageLink, Rotation as ContractRotation } from "@/ipc/generated/contract";
import type { RasterImage } from "@/ipc/raster";

export type DocumentViewHandle = {
  /** Scrolls so that the 1-based `page` is at the top. */
  scrollToPage(page: number): void;
  /** Scrolls so that a search hit is a third of the way down the view. */
  revealHit(hit: Highlight): void;
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
  /** The zoom actually shown, in percent (tells fit modes apart from their result). */
  onEffectiveZoomChange?: (percent: number) => void;
  /** Ctrl+wheel asks for one zoom step in (1) or out (-1), anchored at the cursor. */
  onZoomStep?: (direction: 1 | -1) => void;
  /** Search hits drawn over the pages. */
  highlights?: Highlights;
  /** Where the pages' links come from; without it (demo data) pages have no links. */
  links?: LinkSource;
  /** The pointer or focus is on a link (its description), or left it (null). */
  onLinkHover?: (text: string | null) => void;
  onLinkActivate?: (link: PageLink) => void;
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
/**
 * Extra wait for pages kept around the viewport, so that the pages on screen reach the
 * first-in, first-out render queue first (large zooms take long per page).
 */
const NEARBY_EXTRA_DELAY_MS = 150;
/** Wheel distance (in pixels) for one zoom step; a mouse notch is about 100. */
const WHEEL_STEP_PX = 50;

const UNMEASURED: Viewport = { top: 0, left: 0, width: 0, height: 0 };

const CONTRACT_ROTATION: Record<Rotation, ContractRotation> = { 0: "none", 90: "cw90", 180: "cw180", 270: "cw270" };

function measure(element: HTMLElement | null): Viewport {
  if (!element) return UNMEASURED;
  return {
    top: element.scrollTop,
    left: element.scrollLeft,
    width: element.clientWidth,
    // Before layout (and in tests) the element has no height yet; assume the window's.
    height: element.clientHeight || window.innerHeight,
  };
}

/** Moves the scroll position (outside the component, which must not mutate what it is given). */
function scrollElement(element: HTMLElement, position: { top: number; left?: number }) {
  element.scrollTop = position.top;
  if (position.left !== undefined) element.scrollLeft = position.left;
}

/** What the view looked like at the last commit, to keep the reading position when it changes. */
type Snapshot = { pages: PageSize[]; rotation: Rotation; layout: Layout; width: number };

/**
 * Virtualized pages: the content has the full document size, but only pages near the viewport
 * are mounted and rendered (docs/architecture/rendering.md). Zooming and rotating keep the
 * place under the anchor (the cursor for Ctrl+wheel, otherwise the viewport center) in place.
 */
export function DocumentView({
  pages,
  zoom,
  rotation,
  scrollContainer,
  doc,
  renderer,
  onCurrentPageChange,
  onEffectiveZoomChange,
  onZoomStep,
  highlights,
  links,
  onLinkHover,
  onLinkActivate,
  requestDelayMs = REQUEST_DELAY_MS,
  ref,
}: DocumentViewProps) {
  // Measured after the first commit: measuring during render would see the container before
  // siblings such as the sidebar take their space, and the first renders would use the wrong
  // scale. No page mounts until the viewport is known. (A layout effect would run before the
  // parent's ref to the container is attached.)
  const [measured, setMeasured] = useState<Viewport | null>(null);
  const viewport = measured ?? UNMEASURED;
  /** Where the next zoom is anchored (the Ctrl+wheel cursor); null means the viewport center. */
  const [zoomPoint, setZoomPoint] = useState<ViewportPoint | null>(null);
  /** The layout the scroll position in `measured` refers to. */
  const [base, setBase] = useState<Snapshot | null>(null);
  /** A scroll position to move the scroll bar to (a new object for every move). */
  const [scrollIntent, setScrollIntent] = useState<{ top: number; left: number } | null>(null);
  const zoomStep = useRef(onZoomStep);
  useEffect(() => {
    zoomStep.current = onZoomStep;
  });

  useEffect(() => {
    const element = scrollContainer.current;
    if (!element) return;
    const update = () => setMeasured(measure(element));
    update();
    let wheel = 0;
    const onWheel = (event: WheelEvent) => {
      if (!event.ctrlKey) return;
      // Ours, not the WebView's page zoom.
      event.preventDefault();
      wheel += event.deltaMode === WheelEvent.DOM_DELTA_PIXEL ? event.deltaY : event.deltaY * WHEEL_STEP_PX;
      if (Math.abs(wheel) < WHEEL_STEP_PX) return;
      const rect = element.getBoundingClientRect();
      setZoomPoint({ x: event.clientX - rect.left, y: event.clientY - rect.top });
      zoomStep.current?.(wheel < 0 ? 1 : -1);
      wheel = 0;
    };
    element.addEventListener("scroll", update, { passive: true });
    element.addEventListener("wheel", onWheel, { passive: false });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    observer?.observe(element);
    return () => {
      element.removeEventListener("scroll", update);
      element.removeEventListener("wheel", onWheel);
      observer?.disconnect();
    };
  }, [scrollContainer]);

  const factor = zoomFactor(zoom, pages, rotation, viewport);
  const layout = useMemo(() => layoutPages(pages, rotation, factor), [pages, rotation, factor]);
  const width = contentWidth(layout, viewport.width);

  // Keep the reading position: when the layout changes for the same document (zoom, rotation,
  // a fit mode following the window size), the place under the anchor must stay put. The new
  // scroll position is worked out during render, and state is adjusted during render (React
  // then renders again before committing), so no commit ever pairs the new layout with the old
  // scroll position: the pages being read never unmount and keep their bitmaps.
  const layoutChanged = base !== null && (base.layout !== layout || base.width !== width);
  let anchored: { top: number; left: number } | null = null;
  if (measured && base && layoutChanged && base.pages === pages) {
    const point = zoomPoint ?? { x: measured.width / 2, y: measured.height / 2 };
    const anchor = anchorAt(base.layout, base.width, measured, point);
    if (anchor) {
      anchored = scrollForAnchor(layout, width, rotateAnchor(anchor, base.rotation, rotation), point, measured);
    }
  }
  const shown = anchored ? { ...viewport, ...anchored } : viewport;
  const range = measured ? visibleRange(layout, shown) : { first: 0, last: -1 };
  const onScreen = measured ? visibleRange(layout, shown, 0) : { first: 0, last: -1 };
  const currentPage = currentPageAt(layout, shown);

  if (measured !== null && (base === null || layoutChanged || base.pages !== pages)) {
    setBase({ pages, rotation, layout, width });
    setZoomPoint(null);
    if (anchored) {
      // Our copy of the scroll position changes now; the scroll event arrives later, and another
      // zoom step may come first.
      setMeasured({ ...measured, ...anchored });
      setScrollIntent(anchored);
    }
  }
  useLayoutEffect(() => {
    const element = scrollContainer.current;
    if (scrollIntent && element) scrollElement(element, scrollIntent);
  }, [scrollIntent, scrollContainer]);

  const onPageChange = useRef(onCurrentPageChange);
  const onZoomChange = useRef(onEffectiveZoomChange);
  const onHover = useRef(onLinkHover);
  const onActivate = useRef(onLinkActivate);
  useEffect(() => {
    onPageChange.current = onCurrentPageChange;
    onZoomChange.current = onEffectiveZoomChange;
    onHover.current = onLinkHover;
    onActivate.current = onLinkActivate;
  });
  // Stable, so that a page's links do not take a new render for a new callback.
  const hoverLink = useCallback((text: string | null) => onHover.current?.(text), []);
  const activateLink = useCallback((link: PageLink) => onActivate.current?.(link), []);
  useEffect(() => {
    onPageChange.current?.(currentPage);
  }, [currentPage]);
  useEffect(() => {
    if (measured) onZoomChange.current?.(Math.round(factor * 100));
  }, [factor, measured]);

  useImperativeHandle(
    ref,
    () => ({
      scrollToPage(page) {
        const element = scrollContainer.current;
        if (element) scrollElement(element, { top: scrollTopFor(layout, page) });
      },
      revealHit(hit) {
        const element = scrollContainer.current;
        const position = element && revealScroll(hit, pages, rotation, layout, width, measure(element));
        if (element && position) scrollElement(element, position);
      },
    }),
    [layout, scrollContainer, pages, rotation, width],
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
    const box = layout.boxes[index]!;
    const left = pageLeft(box, width);
    const delay =
      requestDelayMs > 0 && (index < onScreen.first || index > onScreen.last)
        ? requestDelayMs + NEARBY_EXTRA_DELAY_MS
        : requestDelayMs;
    slots.push(
      <PageSlot
        key={index}
        index={index}
        box={box}
        left={left}
        overlay={highlights ? pageOverlay(highlights, index, pages[index]!, rotation, box) : null}
        doc={doc}
        renderer={renderer}
        scale={renderAt}
        // While zooming, nothing new is requested: the scale is about to change again.
        paused={scale !== renderAt}
        rotation={CONTRACT_ROTATION[rotation]}
        requestDelayMs={delay}
      />,
    );
    if (links && doc !== undefined) {
      slots.push(
        <PageLinks
          key={`links-${index}`}
          source={links}
          doc={doc}
          index={index}
          page={pages[index]!}
          rotation={rotation}
          box={box}
          left={left}
          delayMs={delay}
          onHover={hoverLink}
          onActivate={activateLink}
        />,
      );
    }
  }

  return (
    <div className="relative" style={{ width, height: layout.totalHeight }}>
      {slots}
    </div>
  );
}

type SlotState = { kind: "loading" } | { kind: "ready" } | { kind: "failed"; message: string };

type PageSlotProps = {
  index: number;
  box: PageBox;
  left: number;
  doc?: DocumentId;
  renderer?: PageRenderer;
  scale: number;
  paused: boolean;
  rotation: ContractRotation;
  requestDelayMs: number;
  overlay: PageOverlay | null;
};

/** Draws a raster into the canvas at its native resolution; CSS scales it to the page box. */
function draw(canvas: HTMLCanvasElement | null, raster: RasterImage) {
  const context = canvas?.getContext("2d");
  if (!canvas || !context) return;
  canvas.width = raster.width;
  canvas.height = raster.height;
  context.putImageData(new ImageData(raster.pixels, raster.width, raster.height), 0, 0);
}

function PageSlot({ index, box, left, doc, renderer, scale, paused, rotation, requestDelayMs, overlay }: PageSlotProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [state, setState] = useState<SlotState>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!renderer || doc === undefined || paused) return;
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
  }, [renderer, doc, index, scale, paused, rotation, attempt, requestDelayMs]);

  return (
    <div
      id={pageElementId(index + 1)}
      role="img"
      aria-label={strings.canvas.page(index + 1)}
      data-state={state.kind}
      className="absolute flex items-center justify-center overflow-hidden bg-white text-sm text-neutral-400 shadow-sm ring-1 ring-black/10"
      style={{ top: box.top, left, width: box.width, height: box.height }}
    >
      <canvas ref={canvasRef} aria-hidden className="absolute inset-0 size-full" />
      {overlay && (
        // Multiplied like a highlighter pen: the text under a hit stays readable.
        <svg
          aria-hidden
          data-highlights
          className="pointer-events-none absolute inset-0 size-full mix-blend-multiply"
          viewBox={`0 0 ${box.width} ${box.height}`}
          preserveAspectRatio="none"
        >
          {overlay.all.map((points, i) => (
            <polygon key={i} points={points} className="fill-yellow-300" />
          ))}
          {overlay.current.map((points, i) => (
            <polygon
              key={`current-${i}`}
              data-current
              points={points}
              className="fill-none stroke-orange-500"
              strokeWidth={2}
              strokeLinejoin="round"
            />
          ))}
        </svg>
      )}
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

type PageLinksProps = {
  source: LinkSource;
  doc: DocumentId;
  index: number;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  left: number;
  delayMs: number;
  onHover: (text: string | null) => void;
  onActivate: (link: PageLink) => void;
};

/**
 * A page's links as transparent buttons over it (MVP-12). They sit next to the page, not inside
 * it: the page is an image to assistive technology, which would hide them. What a link does
 * when activated is up to the shell; nothing here opens anything.
 */
function PageLinks({ source, doc, index, page, rotation, box, left, delayMs, onHover, onActivate }: PageLinksProps) {
  const [loaded, setLoaded] = useState<{ doc: DocumentId; links: PageLink[] } | null>(null);
  const hovered = useRef(false);

  useEffect(() => {
    let current = true;
    const load = () =>
      source.links(doc, index).then(
        (links) => {
          if (current) setLoaded({ doc, links });
        },
        // No links is better than an error for something that is only an aid.
        () => {},
      );
    // Like renders: pages that only flash by during a fast scroll are not asked.
    const timer = delayMs > 0 ? window.setTimeout(load, delayMs) : undefined;
    if (timer === undefined) void load();
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [source, doc, index, delayMs]);

  // A link that scrolls away under the pointer takes its status bar text with it.
  useEffect(() => {
    const state = hovered;
    return () => {
      if (state.current) onHover(null);
    };
  }, [onHover]);

  const links = loaded?.doc === doc ? loaded.links : [];
  if (links.length === 0) return null;
  const enter = (text: string) => {
    hovered.current = true;
    onHover(text);
  };
  const leave = () => {
    hovered.current = false;
    onHover(null);
  };
  return (
    <div
      data-page-links={index + 1}
      className="pointer-events-none absolute"
      style={{ top: box.top, left, width: box.width, height: box.height }}
    >
      {links.map((link) => {
        const area = rectToBox(link.rect, page, rotation, box);
        const text = linkHoverText(link.target);
        return (
          <button
            key={link.id.index}
            type="button"
            aria-label={text}
            data-link={link.target.kind}
            className="pointer-events-auto absolute cursor-pointer rounded-[2px] outline-offset-1 focus-visible:outline-2 focus-visible:outline-primary"
            style={{ left: area.left, top: area.top, width: area.width, height: area.height }}
            onPointerEnter={() => enter(text)}
            onPointerLeave={leave}
            onFocus={() => enter(text)}
            onBlur={leave}
            onClick={() => onActivate(link)}
          />
        );
      })}
    </div>
  );
}
