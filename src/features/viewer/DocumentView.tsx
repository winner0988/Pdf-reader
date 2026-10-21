import {
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type Ref,
  type RefObject,
} from "react";
import { flushSync } from "react-dom";

import { Button } from "@/components/ui/button";
import type { PageSize, Rotation, Zoom } from "@/features/shell/model";
import {
  anchorAt,
  boxToPage,
  contentWidth,
  currentPageAt,
  fitsWidth,
  layoutPages,
  pageLeft,
  pageNear,
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
import { annotationAt } from "@/features/annotations/model";
import { PageAnnotations } from "@/features/annotations/PageAnnotations";
import { PageDrawing } from "@/features/annotations/PageDrawing";
import { pictureRectAt, redactionRectAt, stampRectAt, type Tool } from "@/features/annotations/tools";
import type { AnnotationSource } from "@/features/annotations/source";
import { PageFields } from "@/features/forms/PageFields";
import type { FormSource } from "@/features/forms/source";
import { linkHoverText } from "@/features/links/text";
import type { LinkSource } from "@/features/links/source";
import { hasSelectedText, selectionQuads, type TextSelection } from "@/features/text/model";
import type { TextSource } from "@/features/text/source";
import { useTextSelection, type PagePoint } from "@/features/text/useTextSelection";
import {
  pageOverlay,
  quadPoints,
  revealScroll,
  type Highlight,
  type Highlights,
  type PageOverlay,
} from "@/features/viewer/highlights";
import {
  covers,
  detailBaseScale,
  detailRegion,
  rasterSize,
  visiblePart,
  type BoxRect,
  type RasterSize,
} from "@/features/viewer/detail";
import { drawRaster, errorCodeOf, type PageRenderer, type RenderJob } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type {
  AnnotationId,
  DocumentId,
  Edit,
  HighlightMark,
  PageAnnotation,
  PageLink,
  PageText,
  Point,
  RasterRegion,
  Rotation as ContractRotation,
} from "@/ipc/generated/contract";

export type DocumentViewHandle = {
  /** Scrolls so that the 1-based `page` is at the top. */
  scrollToPage(page: number): void;
  /** Scrolls so that a search hit is a third of the way down the view. */
  revealHit(hit: Highlight): void;
  /** Whether any text is selected (MVP-15). */
  hasSelection(): boolean;
  /** The selected text, or null when nothing is selected. */
  selectedText(): Promise<string | null>;
  /** Where the selected text is on each page, for the highlighter (B2-07); none when nothing is selected. */
  highlightMarks(): Promise<HighlightMark[]>;
  /** The page under a pointer (client coordinates) and the point on it (page space); null off the pages. */
  pageAt(clientX: number, clientY: number): { page: number; point: Point } | null;
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
  /** Where the pages' text comes from (MVP-15); without it (demo data) no text can be selected. */
  text?: TextSource;
  /** Some text became selected (true), or nothing is selected any more (false). */
  onSelectionChange?: (selected: boolean) => void;
  /** The user tried to select text on a page that has none: a scanned page needs OCR. */
  onNoText?: () => void;
  /**
   * How many times each page's text was recognised since the document opened (B2-10): a page
   * whose count changed asks for its text again. Pages not listed have the count 0.
   */
  textVersions?: ReadonlyMap<number, number>;
  /** A page's text arrived, and whether it was recognised from the page's picture (B2-10). */
  onRecognised?: (pageIndex: number, recognised: boolean) => void;
  /** Where the pages' annotations come from (B2-07); without it (demo data) none are shown. */
  annotations?: AnnotationSource;
  /** Changes an annotation (B2-07); without it (the author does not allow it) none can be changed. */
  onAnnotationEdit?: (edit: Edit) => void;
  /** Where the pages' form fields come from (B2-09); without it (demo data) pages have none. */
  forms?: FormSource;
  /** Changes the document for a field the user filled in; rejects if it could not. Without it the fields are only shown. */
  onFieldEdit?: (edit: Edit) => Promise<void>;
  /** The user entered a field that has scripts: they are not run. */
  onFieldScript?: () => void;
  /** Asks for a note's new text. */
  onEditNote?: (page: number, annotation: PageAnnotation) => void;
  /** Asks for a text box's new words and style (B2-30). */
  onEditTextBox?: (page: number, annotation: PageAnnotation) => void;
  /** The text box tool was used on a page: where (page space), to ask what it says. */
  onNewTextBox?: (page: number, at: Point) => void;
  /**
   * What the pointer does on the pages (B2-08): draws with the pen, or puts a stamp down. Without
   * it (or without `onAnnotationEdit`) it selects text and chooses annotations.
   */
  tool?: Tool;
  /** A stamp was put down: the tool is done. */
  onToolDone?: () => void;
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
/**
 * A page zoomed far in is drawn sharp only around the part on screen (B3-09); after scrolling
 * out of that part, the next one is asked for once the view rests this long.
 */
const DETAIL_DELAY_MS = 100;
/** Wheel distance (in pixels) for one zoom step; a mouse notch is about 100. */
const WHEEL_STEP_PX = 50;
/** How far a press may move and still be a click, not a text selection (B2-07). */
const CLICK_SLOP_PX = 4;

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
  text,
  onSelectionChange,
  onNoText,
  textVersions,
  onRecognised,
  annotations,
  onAnnotationEdit,
  onEditNote,
  onEditTextBox,
  onNewTextBox,
  tool,
  onToolDone,
  forms,
  onFieldEdit,
  onFieldScript,
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

  const rootRef = useRef<HTMLDivElement>(null);
  /** The page nearest to a pointer, and where the pointer is in that page's space. */
  const locate = (clientX: number, clientY: number): PagePoint | null => {
    const root = rootRef.current;
    if (!root) return null;
    const bounds = root.getBoundingClientRect();
    const x = clientX - bounds.left;
    const y = clientY - bounds.top;
    const index = pageNear(layout, y);
    if (index === null) return null;
    const box = layout.boxes[index]!;
    const left = pageLeft(box, width);
    return {
      page: index,
      inside: x >= left && x <= left + box.width && y >= box.top && y <= box.top + box.height,
      point: boxToPage({ x: x - left, y: y - box.top }, pages[index]!, rotation, box),
    };
  };
  const textSelection = useTextSelection({ doc, source: text, locate, scrollContainer, onNoText });
  const { selection } = textSelection;

  // The chosen annotation (B2-07). A click (a press that did not move, so not selecting text) on
  // an annotation chooses it, anywhere else lets it go.
  const [chosen, setChosen] = useState<{ page: number; id: AnnotationId } | null>(null);
  const pressed = useRef<{ x: number; y: number } | null>(null);
  const chooseAt = (event: ReactMouseEvent<HTMLDivElement>) => {
    const from = pressed.current;
    pressed.current = null;
    if (!annotations || doc === undefined || !from) return;
    if (Math.hypot(event.clientX - from.x, event.clientY - from.y) > CLICK_SLOP_PX) return;
    // Links, the page's own buttons and the form's fields keep their clicks.
    if (event.target instanceof Element && event.target.closest("button, input, textarea, select")) return;
    const at = locate(event.clientX, event.clientY);
    const found = at?.inside ? annotationAt(annotations.loaded(doc, at.page) ?? [], at.point) : null;
    setChosen(at && found ? { page: at.page, id: found.id } : null);
  };
  const selected = hasSelectedText(selection);
  const onSelection = useRef(onSelectionChange);
  useEffect(() => {
    onSelection.current = onSelectionChange;
  });
  useEffect(() => {
    onSelection.current?.(selected);
  }, [selected]);

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
      hasSelection: textSelection.hasSelection,
      selectedText: textSelection.selectedText,
      highlightMarks: textSelection.selectedQuads,
      pageAt(clientX, clientY) {
        const at = locate(clientX, clientY);
        return at?.inside ? { page: at.page, point: at.point } : null;
      },
    }),
    // `locate` reads the layout and the view's position, both already listed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [layout, scrollContainer, pages, rotation, width, textSelection.hasSelection, textSelection.selectedText, textSelection.selectedQuads],
  );

  // The first renders use the measured scale right away. After that, a new resolution is
  // rendered only once the scale settles; meanwhile the old bitmap is stretched to the new size.
  const ratio = window.devicePixelRatio;
  const scale = renderScale(factor, ratio);
  const [settled, setSettled] = useState<{ scale: number; factor: number } | null>(null);
  const hasViewport = measured !== null;
  useEffect(() => {
    if (!hasViewport || scale === settled?.scale) return;
    const delay = settled === null ? 0 : RESCALE_DELAY_MS;
    const timer = window.setTimeout(() => setSettled({ scale, factor }), delay);
    return () => window.clearTimeout(timer);
  }, [hasViewport, scale, factor, settled]);
  const renderAt = settled?.scale ?? scale;
  const renderFactor = settled?.factor ?? factor;
  // The screen in device pixels, against which a page counts as zoomed far in (B3-09).
  const deviceRatio = Number.isFinite(ratio) && ratio > 0 ? ratio : 1;
  const screenPixels = { width: viewport.width * deviceRatio, height: viewport.height * deviceRatio };
  const contractRotation = CONTRACT_ROTATION[rotation];

  const slots = [];
  for (let index = range.first; index <= range.last; index++) {
    const box = layout.boxes[index]!;
    const left = pageLeft(box, width);
    const nearby = requestDelayMs > 0 && (index < onScreen.first || index > onScreen.last);
    const delay = nearby ? requestDelayMs + NEARBY_EXTRA_DELAY_MS : requestDelayMs;
    slots.push(
      <PageSlot
        key={index}
        index={index}
        page={pages[index]!}
        box={box}
        left={left}
        view={visiblePart(box, left, shown)}
        baseScale={detailBaseScale(pages[index]!, contractRotation, renderFactor, ratio, screenPixels)}
        overlay={highlights ? pageOverlay(highlights, index, pages[index]!, rotation, box) : null}
        doc={doc}
        renderer={renderer}
        scale={renderAt}
        // While zooming, nothing new is requested: the scale is about to change again.
        paused={scale !== renderAt}
        rotation={contractRotation}
        requestDelayMs={delay}
        redrawDelayMs={nearby ? NEARBY_EXTRA_DELAY_MS : 0}
      />,
    );
    if (text && doc !== undefined) {
      slots.push(
        <PageSelection
          key={`text-${index}`}
          source={text}
          doc={doc}
          index={index}
          version={textVersions?.get(index) ?? 0}
          onRecognised={onRecognised}
          page={pages[index]!}
          rotation={rotation}
          box={box}
          left={left}
          delayMs={delay}
          selection={selection}
        />,
      );
    }
    if (annotations && doc !== undefined) {
      slots.push(
        <PageAnnotations
          key={`annotations-${index}`}
          source={annotations}
          doc={doc}
          index={index}
          page={pages[index]!}
          rotation={rotation}
          box={box}
          left={left}
          delayMs={delay}
          chosen={chosen?.page === index ? chosen.id : null}
          onChoose={(id) => setChosen(id === null ? null : { page: index, id })}
          onRelease={() => {
            setChosen(null);
            scrollContainer.current?.focus();
          }}
          onEdit={onAnnotationEdit}
          onEditNote={onEditNote}
          onEditTextBox={onEditTextBox}
        />,
      );
    }
    if (forms && doc !== undefined) {
      slots.push(
        <PageFields
          key={`fields-${index}`}
          source={forms}
          doc={doc}
          index={index}
          page={pages[index]!}
          rotation={rotation}
          box={box}
          left={left}
          delayMs={delay}
          allowed={onFieldEdit !== undefined}
          onEdit={onFieldEdit ?? (() => Promise.reject(new Error("the fields cannot be changed")))}
          onScript={() => onFieldScript?.()}
        />,
      );
    }
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
    // The last layer, so that it gets the press before links, fields or text do.
    if (tool && onAnnotationEdit && doc !== undefined) {
      slots.push(
        <PageDrawing
          key={`drawing-${index}`}
          tool={tool}
          index={index}
          page={pages[index]!}
          rotation={rotation}
          box={box}
          left={left}
          onStroke={(page, points) => {
            if (tool.kind === "pen") {
              onAnnotationEdit({ kind: "addInk", page, strokes: [points], color: tool.color, width: tool.width });
            }
          }}
          onRect={(page, rect) => {
            if (tool.kind === "redaction") {
              onAnnotationEdit({ kind: "addRedaction", page, rect });
            }
            onToolDone?.();
          }}
          onPlace={(page, at) => {
            if (tool.kind === "stamp") {
              onAnnotationEdit({ kind: "addStamp", page, rect: stampRectAt(at, pages[page]!), stamp: tool.stamp });
            } else if (tool.kind === "picture") {
              onAnnotationEdit({
                kind: "addImageStamp",
                page,
                rect: pictureRectAt(at, pages[page]!, tool.picture),
                image: tool.picture.image,
              });
            } else if (tool.kind === "textBox") {
              onNewTextBox?.(page, at);
            } else if (tool.kind === "redaction") {
              onAnnotationEdit({ kind: "addRedaction", page, rect: redactionRectAt(at, pages[page]!) });
            } else {
              return;
            }
            onToolDone?.();
          }}
        />,
      );
    }
  }

  return (
    <div
      ref={rootRef}
      // Text is selected by the app (MVP-15), not the WebView: it lives in the PDF, not in the DOM.
      className="relative select-none"
      // Content that fits is exactly as wide as the viewport, whatever fraction of a pixel that is.
      style={{ width: fitsWidth(layout, viewport.width) ? "100%" : width, height: layout.totalHeight }}
      onMouseDown={(event) => {
        pressed.current = event.button === 0 ? { x: event.clientX, y: event.clientY } : null;
        textSelection.onMouseDown(event);
      }}
      onMouseMove={textSelection.onMouseMove}
      onClick={chooseAt}
    >
      {slots}
    </div>
  );
}

type SlotState =
  | { kind: "loading" }
  | { kind: "ready"; doc: DocumentId; scale: number; rotation: ContractRotation }
  | { kind: "failed"; message: string };

/** A part of a page drawn sharp over the whole page (B3-09): `region` of the raster `whole`. */
type Detail = {
  doc: DocumentId;
  scale: number;
  rotation: ContractRotation;
  region: RasterRegion;
  whole: RasterSize;
};

type PageSlotProps = {
  index: number;
  page: PageSize;
  box: PageBox;
  left: number;
  /** The part of the page on screen, in its box's pixels; null when none of it is. */
  view: BoxRect | null;
  doc?: DocumentId;
  renderer?: PageRenderer;
  scale: number;
  /**
   * For a page zoomed far in (B3-09), the scale of the whole page underneath the part drawn
   * sharp at `scale`; null when the whole page is drawn at `scale`.
   */
  baseScale: number | null;
  paused: boolean;
  rotation: ContractRotation;
  /** Wait before the first render of a newly mounted page. */
  requestDelayMs: number;
  /** Wait before a page already drawn is drawn again at a new scale or rotation. */
  redrawDelayMs: number;
  overlay: PageOverlay | null;
};

function PageSlot({
  index,
  page,
  box,
  left,
  view,
  doc,
  renderer,
  scale,
  baseScale,
  paused,
  rotation,
  requestDelayMs,
  redrawDelayMs,
  overlay,
}: PageSlotProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // What the canvas has in it, which is stretched until the next one comes; null after a failure.
  const drawnAt = useRef<{ doc: DocumentId; scale: number; rotation: ContractRotation } | null>(null);
  const [state, setState] = useState<SlotState>({ kind: "loading" });
  const [attempt, setAttempt] = useState(0);

  // The whole page: at `scale`, or underneath the part drawn sharp at a lower one.
  const wholeAt = baseScale ?? scale;
  useEffect(() => {
    if (!renderer || doc === undefined || paused) return;
    // Already drawn so, such as at the zoom the view had before zooming in far.
    const drawn = drawnAt.current;
    if (drawn && drawn.doc === doc && drawn.scale === wholeAt && drawn.rotation === rotation) return;
    let job: RenderJob | null = null;
    const start = () => {
      job = renderer.render({ doc, pageIndex: index, scale: wholeAt, rotation });
      job.result.then(
        (raster) => {
          drawRaster(canvasRef.current, raster);
          drawnAt.current = { doc, scale: wholeAt, rotation };
          setState({ kind: "ready", doc, scale: wholeAt, rotation });
        },
        (error: unknown) => {
          const code = errorCodeOf(error);
          // Cancelled: the page left the view or the view changed. Unknown document: it was closed.
          if (code === "cancelled" || code === "unknownDocument") return;
          drawnAt.current = null;
          setState({ kind: "failed", message: strings.canvas.pageRenderFailed });
        },
      );
    };
    // A page on screen is drawn again at once after zooming or turning: the delay is for pages
    // that only flash by while scrolling. A page of another document is new.
    const delay = drawn?.doc === doc ? redrawDelayMs : requestDelayMs;
    const timer = delay > 0 ? window.setTimeout(start, delay) : undefined;
    if (timer === undefined) start();
    // Leaving the rendered range, or a new scale or rotation, cancels the old request.
    return () => {
      window.clearTimeout(timer);
      (job as RenderJob | null)?.cancel();
    };
  }, [renderer, doc, index, wholeAt, paused, rotation, attempt, requestDelayMs, redrawDelayMs]);

  // Zoomed far in (B3-09): the part on screen, sharp, over the whole page. A new part is asked
  // for when the view leaves the one asked for last; it replaces the old one once drawn.
  const detailRef = useRef<HTMLCanvasElement>(null);
  const [asked, setAsked] = useState<Detail | null>(null);
  const [detail, setDetail] = useState<Detail | null>(null);
  const whole = rasterSize(page, rotation, scale);
  const onScreen = baseScale !== null && view ? detailRegion(view, box, whole, 0) : null;
  const sharpIn = (part: Detail | null) =>
    part !== null &&
    part.doc === doc &&
    part.scale === scale &&
    part.rotation === rotation &&
    onScreen !== null &&
    covers(part.region, onScreen);
  const region = !paused && onScreen && view && !sharpIn(asked) ? detailRegion(view, box, whole) : null;
  const wanted: Detail | null =
    region && doc !== undefined ? { doc, scale, rotation, region, whole } : null;
  // Comparable between renders, so that only a new part restarts the wait.
  const wantedKey = wanted && JSON.stringify(wanted);
  // A part asked for at another zoom or turn is no longer wanted once the view has settled.
  const stale = asked !== null && (asked.doc !== doc || asked.scale !== scale || asked.rotation !== rotation);
  // Right after zooming, at once; while scrolling, once the view rests.
  const scrolling = asked !== null && !stale;
  const drawnHere = state.kind === "ready" && state.doc === doc;
  const detailDelay = scrolling ? DETAIL_DELAY_MS : drawnHere ? 0 : requestDelayMs;
  useEffect(() => {
    if (!wantedKey) return;
    const timer = window.setTimeout(() => setAsked(JSON.parse(wantedKey) as Detail), detailDelay);
    return () => window.clearTimeout(timer);
  }, [wantedKey, detailDelay]);
  useEffect(() => {
    if (!renderer || !asked || stale) return;
    let live = true;
    const job = renderer.render({
      doc: asked.doc,
      pageIndex: index,
      scale: asked.scale,
      rotation: asked.rotation,
      region: asked.region,
    });
    job.result.then(
      (raster) => {
        if (!live) return;
        // Moved into place first, in the same task: no frame shows it where the old part was.
        flushSync(() => setDetail(asked));
        drawRaster(detailRef.current, raster);
      },
      // Failed or cancelled: the whole page underneath stays until the view leaves this part or
      // the zoom changes. Asking again at once would loop on an error that stays.
      () => {},
    );
    return () => {
      live = false;
      job.cancel();
    };
  }, [renderer, index, asked, stale]);

  const wholeSharp = drawnHere && state.scale === scale && state.rotation === rotation;
  // Shown while zoomed far in, and after zooming out until the whole page is drawn sharp.
  const shownDetail =
    detail !== null && detail.doc === doc && detail.rotation === rotation && (baseScale !== null || !wholeSharp)
      ? detail
      : null;
  const sharp = !paused && (baseScale === null ? wholeSharp : sharpIn(detail));
  const percent = (part: number, of: number) => `${(part / of) * 100}%`;

  return (
    <div
      role="img"
      aria-label={strings.canvas.page(index + 1)}
      data-state={state.kind}
      data-doc={doc}
      // For measuring (scripts/perf): the part on screen is drawn at the current zoom.
      data-sharp={sharp ? "" : undefined}
      className="absolute flex items-center justify-center overflow-hidden bg-white text-sm text-neutral-400 shadow-sm ring-1 ring-black/10"
      style={{ top: box.top, left, width: box.width, height: box.height }}
    >
      <canvas ref={canvasRef} aria-hidden className="absolute inset-0 size-full" />
      <canvas
        ref={detailRef}
        aria-hidden
        data-detail
        hidden={shownDetail === null}
        className="absolute"
        style={
          shownDetail
            ? {
                left: percent(shownDetail.region.x, shownDetail.whole.width),
                top: percent(shownDetail.region.y, shownDetail.whole.height),
                width: percent(shownDetail.region.width, shownDetail.whole.width),
                height: percent(shownDetail.region.height, shownDetail.whole.height),
              }
            : undefined
        }
      />
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

type PageSelectionProps = {
  source: TextSource;
  doc: DocumentId;
  index: number;
  /** See `DocumentViewProps.textVersions`. */
  version: number;
  onRecognised?: (pageIndex: number, recognised: boolean) => void;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  left: number;
  delayMs: number;
  selection: TextSelection | null;
};

/**
 * A page's text (MVP-15): asked for once the page has been in view for a moment, so that the
 * pointer can select on it, and the selected part of it drawn over the page.
 */
function PageSelection({
  source,
  doc,
  index,
  version,
  onRecognised,
  page,
  rotation,
  box,
  left,
  delayMs,
  selection,
}: PageSelectionProps) {
  const [loaded, setLoaded] = useState<{ doc: DocumentId; text: PageText } | null>(null);
  // The latest, for the effect below, which must not run again for a new callback.
  const told = useRef(onRecognised);
  useEffect(() => {
    told.current = onRecognised;
  });

  useEffect(() => {
    let current = true;
    const load = () =>
      source.text(doc, index, version).then(
        (text) => {
          if (!current) return;
          setLoaded({ doc, text });
          told.current?.(index, text.recognised);
        },
        // Without its text a page only cannot be selected; the next time it is shown asks again.
        () => {},
      );
    // Like renders: pages that only flash by during a fast scroll are not asked.
    const timer = delayMs > 0 ? window.setTimeout(load, delayMs) : undefined;
    if (timer === undefined) void load();
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [source, doc, index, version, delayMs]);

  const text = loaded?.doc === doc ? loaded.text : undefined;
  const quads = text && hasSelectedText(selection) ? selectionQuads(text, index, selection) : [];
  if (quads.length === 0) return null;
  return (
    <svg
      aria-hidden
      data-selection={index + 1}
      // Multiplied like the search highlights: the text under the selection stays readable.
      className="pointer-events-none absolute mix-blend-multiply"
      style={{ top: box.top, left, width: box.width, height: box.height }}
      viewBox={`0 0 ${box.width} ${box.height}`}
      preserveAspectRatio="none"
    >
      {quads.map((quad, i) => (
        <polygon key={i} points={quadPoints(quad, page, rotation, box)} className="fill-sky-500/35" />
      ))}
    </svg>
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
