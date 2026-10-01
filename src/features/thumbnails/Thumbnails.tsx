import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
} from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuGroup,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import type { PageSize } from "@/features/shell/model";
import {
  ITEM_GAP,
  layoutThumbs,
  scrollToShow,
  visibleThumbs,
  type ThumbBox,
  type ThumbLayout,
} from "@/features/thumbnails/layout";
import { MovePagesDialog } from "@/features/thumbnails/MovePagesDialog";
import {
  extendTo,
  movesNothing,
  NO_SELECTION,
  selectAll,
  selectionAfter,
  selectOnly,
  toggle,
  within,
  type Selection,
} from "@/features/thumbnails/selection";
import { CSS_PX_PER_PT, renderScale } from "@/features/viewer/layout";
import { drawRaster, errorCodeOf, type PageRenderer, type RenderJob } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, Edit } from "@/ipc/generated/contract";

const t = strings.pages;

/** Page management in the thumbnails (B2-05). */
export type PageEditing = {
  /** The document's author allows assembling or changing it (MVP-19). */
  allowed: boolean;
  /** Applies `edit` to the document shown; resolves once the document has it. */
  apply: (edit: Edit) => Promise<void>;
};

type ThumbnailsProps = {
  pages: PageSize[];
  /** Without a document id or renderer (demo data), thumbnails stay blank. */
  doc?: DocumentId;
  renderer?: PageRenderer;
  currentPage: number;
  onJumpToPage: (page: number) => void;
  /** Page management; without it (demo data) pages can only be looked at. */
  editing?: PageEditing;
  /** Delay before a thumbnail that came into view asks for a render; tests pass 0. */
  requestDelayMs?: number;
};

/** Like the pages: thumbnails that only flash by while scrolling are never rendered. */
const REQUEST_DELAY_MS = 100;

/** How far the pointer moves before pressing a thumbnail becomes dragging it. */
const DRAG_THRESHOLD_PX = 6;
/** Near the list's top or bottom edge, dragging scrolls the list this much per pointer move. */
const DRAG_SCROLL_EDGE_PX = 32;
const DRAG_SCROLL_STEP_PX = 24;

/** A press on a thumbnail, and once it moved far enough, where its pages would go. */
type Drag = { pointerId: number; page: number; x: number; y: number; before: number | null };

/** Where a drop at `y` (list coordinates) puts pages: before the first thumbnail below it. */
function dropBefore(layout: ThumbLayout, y: number): number {
  const index = layout.boxes.findIndex((box) => y < box.top + box.height / 2);
  return index === -1 ? layout.boxes.length : index;
}

/**
 * Page thumbnails (MVP-18): a virtualized list, rendered small by the worker once a thumbnail
 * has been in view for a moment. Clicking one goes to its page; the page being read is marked
 * and kept in view. Up and down arrows move between thumbnails, Enter goes to one.
 *
 * Page management (B2-05): Ctrl and Shift select several; the context menu turns, deletes and
 * inserts pages and moves them to a page number; dragging moves them; Delete deletes them.
 */
export function Thumbnails({
  pages,
  doc,
  renderer,
  currentPage,
  onJumpToPage,
  editing,
  requestDelayMs = REQUEST_DELAY_MS,
}: ThumbnailsProps) {
  const scroller = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 0 });
  /** A thumbnail to move the focus to once it is rendered. */
  const [focusing, setFocusing] = useState<number | null>(null);
  /** The thumbnail the list's Tab stop is on. */
  const [active, setActive] = useState(Math.max(currentPage - 1, 0));
  const [selection, setSelection] = useState<Selection>(NO_SELECTION);
  /** An edit on its way: the next waits, as its pages are counted on this one's result. */
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [moveOpen, setMoveOpen] = useState(false);
  const [drag, setDrag] = useState<Drag | null>(null);
  /** The click that ends a drag selects nothing. */
  const dragged = useRef(false);
  const layout = useMemo(() => layoutThumbs(pages), [pages]);
  const count = pages.length;

  // Deleted pages leave the selection.
  const kept = within(selection, count);
  if (kept !== selection) setSelection(kept);
  if (active >= count && count > 0) setActive(count - 1);

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
    const option = scroller.current?.querySelector<HTMLElement>(`[data-index="${focusing}"]`);
    if (!option) return;
    option.focus();
    setFocusing(null);
  }, [focusing, range.first, range.last]);

  /** Scrolls thumbnail `index` into view and moves the focus there. */
  const focusThumb = (index: number) => {
    const element = scroller.current;
    if (!element) return;
    element.scrollTop = scrollToShow(layout, index, element.scrollTop, element.clientHeight || window.innerHeight);
    setView({ top: element.scrollTop, height: element.clientHeight || window.innerHeight });
    setActive(index);
    setFocusing(index);
  };

  const canEdit = editing !== undefined && editing.allowed && !busy && doc !== undefined;
  const selected = selection.pages;
  const allSelected = selected.length >= count;

  const apply = (edit: Edit) => {
    if (!editing || !canEdit) return;
    setBusy(true);
    setMessage(null);
    editing.apply(edit).then(
      () => {
        setBusy(false);
        setSelection(selectionAfter(edit));
      },
      () => {
        setBusy(false);
        setMessage(t.failed);
      },
    );
  };

  /** Deletes `targets` (the selected pages), unless that would leave none. */
  const deletePages = (targets: number[] = selected) => {
    if (!canEdit || targets.length === 0) return;
    if (targets.length >= count) {
      setMessage(t.keepOne);
      return;
    }
    apply({ kind: "deletePages", pages: targets });
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = Number((event.target as HTMLElement).dataset.index);
    if (Number.isNaN(index)) return;
    const moveFocus = (to: number) => {
      if (to < 0 || to >= count) return;
      event.preventDefault();
      if (event.shiftKey) {
        setSelection((current) => extendTo(current.anchor === null ? selectOnly(index) : current, to));
      }
      focusThumb(to);
    };
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
      event.preventDefault();
      setSelection(selectAll(count));
      return;
    }
    switch (event.key) {
      case "ArrowDown":
        moveFocus(index + 1);
        break;
      case "ArrowUp":
        moveFocus(index - 1);
        break;
      case "Home":
        moveFocus(0);
        break;
      case "End":
        moveFocus(count - 1);
        break;
      case "Enter":
        event.preventDefault();
        setSelection(selectOnly(index));
        onJumpToPage(index + 1);
        break;
      case " ":
        event.preventDefault();
        setSelection((current) => toggle(current, index));
        break;
      case "Delete":
        event.preventDefault();
        // With nothing selected, the page with the focus.
        deletePages(selected.length > 0 ? selected : [index]);
        break;
      case "Escape":
        if (drag) {
          event.preventDefault();
          setDrag(null);
        }
        break;
    }
  };

  const onClick = (event: MouseEvent, index: number) => {
    if (dragged.current) {
      dragged.current = false;
      return;
    }
    setActive(index);
    if (event.ctrlKey || event.metaKey) {
      setSelection((current) => toggle(current, index));
    } else if (event.shiftKey) {
      setSelection((current) => extendTo(current, index));
    } else {
      setSelection(selectOnly(index));
      onJumpToPage(index + 1);
    }
  };

  /** Right-clicking a thumbnail outside the selection selects it alone, as the menu acts on it. */
  const onContextMenu = (index: number) => {
    setActive(index);
    if (!selected.includes(index)) setSelection(selectOnly(index));
  };

  // Dragging moves pages (pointer events: the window's own file drop takes the WebView's drag
  // and drop events).
  const onPointerDown = (event: PointerEvent, index: number) => {
    // The click after a drag lands on the list, not on a thumbnail: a new press starts afresh.
    dragged.current = false;
    if (!canEdit || event.button !== 0 || event.ctrlKey || event.shiftKey || event.metaKey) return;
    setDrag({ pointerId: event.pointerId, page: index, x: event.clientX, y: event.clientY, before: null });
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    if (drag.before === null) {
      if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < DRAG_THRESHOLD_PX) return;
      event.currentTarget.setPointerCapture?.(event.pointerId);
      // Dragging a page outside the selection drags it alone.
      if (!selected.includes(drag.page)) setSelection(selectOnly(drag.page));
    }
    const element = scroller.current;
    const listTop = list.current?.getBoundingClientRect().top;
    if (!element || listTop === undefined) return;
    const bounds = element.getBoundingClientRect();
    if (event.clientY < bounds.top + DRAG_SCROLL_EDGE_PX) element.scrollTop -= DRAG_SCROLL_STEP_PX;
    else if (event.clientY > bounds.bottom - DRAG_SCROLL_EDGE_PX) element.scrollTop += DRAG_SCROLL_STEP_PX;
    setDrag({ ...drag, before: dropBefore(layout, event.clientY - listTop) });
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag || event.pointerId !== drag.pointerId) return;
    setDrag(null);
    if (drag.before === null) return;
    dragged.current = true;
    const moving = selected.includes(drag.page) ? selected : [drag.page];
    if (!movesNothing(moving, drag.before)) apply({ kind: "movePages", pages: moving, before: drag.before });
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
        selected={selected.includes(index)}
        tabbable={index === active}
        delayMs={requestDelayMs}
        onClick={onClick}
        onContextMenu={onContextMenu}
        onPointerDown={onPointerDown}
      />,
    );
  }

  const dropLine =
    drag?.before !== null && drag?.before !== undefined
      ? (layout.boxes[drag.before]?.top ?? layout.totalHeight) - ITEM_GAP / 2
      : null;

  const listbox = (
    <div
      ref={list}
      role="listbox"
      aria-label={strings.sidebar.thumbnailsTab}
      aria-multiselectable="true"
      aria-busy={busy || undefined}
      className="relative"
      style={{ height: layout.totalHeight }}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => setDrag(null)}
    >
      {items}
      {dropLine !== null && (
        <div aria-hidden className="absolute inset-x-3 h-0.5 rounded-full bg-primary" style={{ top: dropLine - 1 }} />
      )}
    </div>
  );

  return (
    <div className="flex h-full flex-col">
      {selected.length > 1 && (
        <p className="px-3 pt-1 text-xs text-muted-foreground">{t.selected(selected.length)}</p>
      )}
      {message && (
        <p role="alert" className="mx-2 mt-1 rounded-md bg-muted px-2 py-1.5 text-xs">
          {message}
        </p>
      )}
      <div ref={scroller} className="min-h-0 flex-1 overflow-auto" onKeyDown={onKeyDown}>
        {editing ? (
          <ContextMenu>
            <ContextMenuTrigger>{listbox}</ContextMenuTrigger>
            <ContextMenuContent>
              <ContextMenuGroup>
                {!editing.allowed && <ContextMenuLabel>{t.notAllowed}</ContextMenuLabel>}
                <ContextMenuItem
                  disabled={!canEdit || selected.length === 0}
                  onClick={() => apply({ kind: "rotatePages", pages: selected, by: "cw90" })}
                >
                  {t.rotateCw}
                </ContextMenuItem>
                <ContextMenuItem
                  disabled={!canEdit || selected.length === 0}
                  onClick={() => apply({ kind: "rotatePages", pages: selected, by: "cw270" })}
                >
                  {t.rotateCcw}
                </ContextMenuItem>
                <ContextMenuItem disabled={!canEdit || selected.length === 0 || allSelected} onClick={() => deletePages()}>
                  {t.delete}
                  <ContextMenuShortcut>Delete</ContextMenuShortcut>
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem
                  disabled={!canEdit || selected.length === 0}
                  onClick={() => apply({ kind: "insertBlankPage", at: selected[0]!, like: selected[0]! })}
                >
                  {t.insertBefore}
                </ContextMenuItem>
                <ContextMenuItem
                  disabled={!canEdit || selected.length === 0}
                  onClick={() => apply({ kind: "insertBlankPage", at: selected.at(-1)! + 1, like: selected.at(-1)! })}
                >
                  {t.insertAfter}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem disabled={!canEdit || selected.length === 0} onClick={() => setMoveOpen(true)}>
                  {t.moveTo}
                </ContextMenuItem>
              </ContextMenuGroup>
            </ContextMenuContent>
          </ContextMenu>
        ) : (
          listbox
        )}
      </div>
      {editing && (
        <MovePagesDialog
          open={moveOpen}
          onOpenChange={setMoveOpen}
          pageCount={count}
          moving={selected.length}
          onMove={(before) => {
            if (!movesNothing(selected, before)) apply({ kind: "movePages", pages: selected, before });
          }}
        />
      )}
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
  selected: boolean;
  /** The list's Tab stop. */
  tabbable: boolean;
  delayMs: number;
  onClick: (event: MouseEvent, index: number) => void;
  onContextMenu: (index: number) => void;
  onPointerDown: (event: PointerEvent, index: number) => void;
};

function Thumbnail({
  index,
  box,
  page,
  doc,
  renderer,
  current,
  selected,
  tabbable,
  delayMs,
  onClick,
  onContextMenu,
  onPointerDown,
}: ThumbnailProps) {
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

  return (
    <div
      role="option"
      data-index={index}
      aria-label={strings.canvas.page(index + 1)}
      aria-selected={selected}
      aria-current={current ? "page" : undefined}
      tabIndex={tabbable ? 0 : -1}
      title={failed ? strings.canvas.pageRenderFailed : undefined}
      className="group absolute inset-x-2 flex cursor-default flex-col items-center rounded-md py-1 outline-none select-none focus-visible:outline-2 focus-visible:outline-primary aria-selected:bg-primary/15"
      style={{ top: box.top - 4 }}
      onClick={(event) => onClick(event, index)}
      onContextMenu={() => onContextMenu(index)}
      onPointerDown={(event) => onPointerDown(event, index)}
    >
      <div
        className="rounded-sm bg-white shadow-sm ring-1 ring-black/10 group-hover:ring-2 group-hover:ring-primary/50 group-aria-[current=page]:ring-3 group-aria-[current=page]:ring-primary"
        style={{ width: box.width, height: box.height }}
      >
        <canvas ref={canvasRef} aria-hidden className="size-full rounded-sm" />
      </div>
      <span aria-hidden className="mt-1 text-xs text-muted-foreground">
        {index + 1}
      </span>
    </div>
  );
}
