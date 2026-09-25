import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";

import type { PageSize } from "@/features/shell/model";
import {
  layoutThumbs,
  scrollToShow,
  visibleThumbs,
  type ThumbBox,
} from "@/features/thumbnails/layout";
import { CSS_PX_PER_PT, renderScale } from "@/features/viewer/layout";
import { drawRaster, errorCodeOf, type PageRenderer, type RenderJob } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId } from "@/ipc/generated/contract";

type ThumbnailsProps = {
  pages: PageSize[];
  /** Without a document id or renderer (demo data), thumbnails stay blank. */
  doc?: DocumentId;
  renderer?: PageRenderer;
  currentPage: number;
  onJumpToPage: (page: number) => void;
  /** Delay before a thumbnail that came into view asks for a render; tests pass 0. */
  requestDelayMs?: number;
};

/** Like the pages: thumbnails that only flash by while scrolling are never rendered. */
const REQUEST_DELAY_MS = 100;

/**
 * Page thumbnails (MVP-18): a virtualized list, rendered small by the worker once a thumbnail
 * has been in view for a moment. Clicking one goes to its page; the page being read is marked
 * and kept in view. Up and down arrows move between thumbnails.
 */
export function Thumbnails({
  pages,
  doc,
  renderer,
  currentPage,
  onJumpToPage,
  requestDelayMs = REQUEST_DELAY_MS,
}: ThumbnailsProps) {
  const scroller = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 0 });
  /** A thumbnail to move the focus to once it is rendered. */
  const [focusing, setFocusing] = useState<number | null>(null);
  const layout = useMemo(() => layoutThumbs(pages), [pages]);

  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    // Before layout (and in tests) the list has no height yet; assume the window's.
    const update = () => setView({ top: element.scrollTop, height: element.clientHeight || window.innerHeight });
    update();
    element.addEventListener("scroll", update, { passive: true });
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    observer?.observe(element);
    return () => {
      element.removeEventListener("scroll", update);
      observer?.disconnect();
    };
  }, []);

  // The page being read stays in view.
  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const top = scrollToShow(layout, currentPage - 1, element.scrollTop, element.clientHeight || window.innerHeight);
    if (top !== element.scrollTop) element.scrollTop = top;
  }, [layout, currentPage]);

  const range = visibleThumbs(layout, view.top, view.height);

  useEffect(() => {
    if (focusing === null) return;
    const button = scroller.current?.querySelector<HTMLElement>(`[data-index="${focusing}"]`);
    if (!button) return;
    button.focus();
    setFocusing(null);
  }, [focusing, range.first, range.last]);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = Number((event.target as HTMLElement).dataset.index);
    const element = scroller.current;
    if (Number.isNaN(index) || !element) return;
    const next = event.key === "ArrowDown" ? index + 1 : event.key === "ArrowUp" ? index - 1 : null;
    if (next === null || next < 0 || next >= pages.length) return;
    event.preventDefault();
    element.scrollTop = scrollToShow(layout, next, element.scrollTop, element.clientHeight || window.innerHeight);
    setView({ top: element.scrollTop, height: element.clientHeight || window.innerHeight });
    setFocusing(next);
  };

  const items = [];
  for (let index = range.first; index <= range.last; index++) {
    items.push(
      <Thumbnail
        key={index}
        index={index}
        box={layout.boxes[index]!}
        page={pages[index]!}
        doc={doc}
        renderer={renderer}
        current={index === currentPage - 1}
        delayMs={requestDelayMs}
        onJump={onJumpToPage}
      />,
    );
  }

  return (
    <div ref={scroller} className="h-full overflow-auto" onKeyDown={onKeyDown}>
      <div role="list" aria-label={strings.sidebar.thumbnailsTab} className="relative" style={{ height: layout.totalHeight }}>
        {items}
      </div>
    </div>
  );
}

type ThumbnailProps = {
  index: number;
  box: ThumbBox;
  page: PageSize;
  doc?: DocumentId;
  renderer?: PageRenderer;
  current: boolean;
  delayMs: number;
  onJump: (page: number) => void;
};

function Thumbnail({ index, box, page, doc, renderer, current, delayMs, onJump }: ThumbnailProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!renderer || doc === undefined) return;
    // Sharp on the screen: as many device pixels as the thumbnail covers.
    const scale = renderScale(box.width / (page.widthPt * CSS_PX_PER_PT), window.devicePixelRatio);
    let job: RenderJob | null = null;
    const start = () => {
      job = renderer.render({ doc, pageIndex: index, scale, rotation: "none" });
      job.result.then(
        (raster) => drawRaster(canvasRef.current, raster),
        (error: unknown) => {
          const code = errorCodeOf(error);
          if (code !== "cancelled" && code !== "unknownDocument") setFailed(true);
        },
      );
    };
    const timer = delayMs > 0 ? window.setTimeout(start, delayMs) : undefined;
    if (timer === undefined) start();
    return () => {
      window.clearTimeout(timer);
      (job as RenderJob | null)?.cancel();
    };
  }, [renderer, doc, index, box.width, page.widthPt, delayMs]);

  const label = strings.canvas.page(index + 1);
  return (
    <div role="listitem" className="absolute inset-x-0 flex flex-col items-center" style={{ top: box.top }}>
      <button
        type="button"
        data-index={index}
        aria-label={label}
        aria-current={current ? "page" : undefined}
        className="rounded-sm bg-white shadow-sm ring-1 ring-black/10 outline-offset-2 hover:ring-2 hover:ring-primary/50 focus-visible:outline-2 focus-visible:outline-primary aria-[current=page]:ring-3 aria-[current=page]:ring-primary"
        style={{ width: box.width, height: box.height }}
        title={failed ? strings.canvas.pageRenderFailed : undefined}
        onClick={() => onJump(index + 1)}
      >
        <canvas ref={canvasRef} aria-hidden className="size-full rounded-sm" />
      </button>
      <span aria-hidden className="mt-1 text-xs text-muted-foreground">
        {index + 1}
      </span>
    </div>
  );
}
