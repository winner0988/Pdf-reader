// Selecting text with the mouse (MVP-15, docs/architecture/text-selection.md): press on a line
// and drag (the view scrolls when the pointer reaches its edge), double click for a word, triple
// click for a line, Shift+click to extend. The pages draw the selection; copying asks for its text.

import { useEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type RefObject } from "react";

import {
  caretAt,
  dragSelection,
  hasSelectedText,
  lineAt,
  sameSelection,
  selectedText,
  selectionRange,
  spotAt,
  TEXT_SLACK_PT,
  wordAt,
  type DragStart,
  type TextSelection,
} from "@/features/text/model";
import type { TextSource } from "@/features/text/source";
import type { DocumentId, PageText, Point } from "@/ipc/generated/contract";

/** A pointer position as a page (the nearest one) and a point in that page's space. */
export type PagePoint = { page: number; point: Point; inside: boolean };

type Options = {
  doc?: DocumentId;
  source?: TextSource;
  /** Where a pointer (client coordinates) is on the pages. */
  locate: (clientX: number, clientY: number) => PagePoint | null;
  scrollContainer: RefObject<HTMLElement | null>;
  /** The user tried to select on a page that has no text (a scanned page). */
  onNoText?: () => void;
};

export type TextSelectionController = {
  selection: TextSelection | null;
  onMouseDown(event: ReactMouseEvent<HTMLElement>): void;
  /** Shows the text cursor over text. */
  onMouseMove(event: ReactMouseEvent<HTMLElement>): void;
  hasSelection(): boolean;
  /** The selected text, reading the pages it spans; null when nothing is selected. */
  selectedText(): Promise<string | null>;
};

/** Within this distance (CSS px) of the view's edge a drag scrolls, at most this far per frame. */
const EDGE_PX = 24;
const MAX_STEP_PX = 40;
/** How far the mouse must move before a press counts as a drag. */
const DRAG_THRESHOLD_PX = 4;

/** How far to scroll for a pointer at `position` in a view from `low` to `high`. */
function edgeScroll(position: number, low: number, high: number): number {
  if (position < low + EDGE_PX) return -Math.min(MAX_STEP_PX, Math.ceil((low + EDGE_PX - position) / 2));
  if (position > high - EDGE_PX) return Math.min(MAX_STEP_PX, Math.ceil((position - (high - EDGE_PX)) / 2));
  return 0;
}

const nextFrame = (callback: () => void): number =>
  typeof window.requestAnimationFrame === "function"
    ? window.requestAnimationFrame(callback)
    : window.setTimeout(callback, 16);
const cancelFrame = (frame: number) => {
  if (typeof window.cancelAnimationFrame === "function") window.cancelAnimationFrame(frame);
  else window.clearTimeout(frame);
};

type Drag = {
  /** Null on a page without text: the drag selects nothing, but says so once it moves. */
  start: DragStart | null;
  from: { x: number; y: number };
  pointer: { x: number; y: number };
  told: boolean;
  frame: number;
  stop: () => void;
};

export function useTextSelection(options: Options): TextSelectionController {
  const { doc, source } = options;
  const [selection, setSelection] = useState<TextSelection | null>(null);
  // Another document starts with nothing selected.
  const [selectionDoc, setSelectionDoc] = useState(doc);
  if (selectionDoc !== doc) {
    setSelectionDoc(doc);
    setSelection(null);
  }

  // Window listeners and animation frames outlive a render: they read the latest of these.
  const latest = useRef({ options, selection });
  useEffect(() => {
    latest.current = { options, selection };
  });
  const drag = useRef<Drag | null>(null);
  useEffect(() => () => drag.current?.stop(), []);

  /** The page's text under a pointer, if it has arrived. */
  const textAt = (clientX: number, clientY: number, onPageOnly: boolean) => {
    const { locate, source: text, doc: document } = latest.current.options;
    const at = locate(clientX, clientY);
    if (!at || !text || document === undefined || (onPageOnly && !at.inside)) return null;
    const pageText: PageText | undefined = text.loaded(document, at.page);
    return pageText ? { at, pageText } : null;
  };

  /** Moves the selection's focus to where the pointer is now. */
  const follow = (current: Drag) => {
    if (!current.start) return;
    const found = textAt(current.pointer.x, current.pointer.y, false);
    const spot = found && spotAt(found.pageText, found.at.page, found.at.point);
    if (!found || !spot) return;
    const next = dragSelection(current.start, found.pageText, spot);
    setSelection((previous) => (sameSelection(previous, next) ? previous : next));
  };

  const startDrag = (clientX: number, clientY: number, start: DragStart | null) => {
    drag.current?.stop();
    const current: Drag = {
      start,
      from: { x: clientX, y: clientY },
      pointer: { x: clientX, y: clientY },
      told: false,
      frame: 0,
      stop: () => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
        cancelFrame(current.frame);
        if (drag.current === current) drag.current = null;
      },
    };
    function move(event: MouseEvent) {
      current.pointer = { x: event.clientX, y: event.clientY };
      if (current.start) {
        follow(current);
      } else if (
        !current.told &&
        Math.hypot(event.clientX - current.from.x, event.clientY - current.from.y) > DRAG_THRESHOLD_PX
      ) {
        current.told = true;
        latest.current.options.onNoText?.();
      }
    }
    function up() {
      current.stop();
    }
    // Every frame: scroll when the pointer is at the view's edge, and catch up with text that
    // arrived for a page that just came into view.
    function tick() {
      const scroller = latest.current.options.scrollContainer.current;
      if (scroller) {
        const view = scroller.getBoundingClientRect();
        const dy = edgeScroll(current.pointer.y, view.top, view.bottom);
        const dx = edgeScroll(current.pointer.x, view.left, view.right);
        if (dy !== 0) scroller.scrollTop += dy;
        if (dx !== 0) scroller.scrollLeft += dx;
      }
      follow(current);
      current.frame = nextFrame(tick);
    }
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    if (start) current.frame = nextFrame(tick);
    drag.current = current;
  };

  return {
    selection,
    onMouseDown(event) {
      if (event.button !== 0 || !source || doc === undefined) return;
      // Links and the retry button keep their clicks.
      if (event.target instanceof Element && event.target.closest("button")) return;
      const found = textAt(event.clientX, event.clientY, true);
      const spot = found && spotAt(found.pageText, found.at.page, found.at.point, TEXT_SLACK_PT);
      if (!found || !spot) {
        setSelection(null);
        if (found && found.pageText.lines.length === 0) startDrag(event.clientX, event.clientY, null);
        return;
      }
      const { pageText } = found;
      let start: DragStart;
      if (event.detail >= 3) start = { unit: "line", first: lineAt(pageText, spot) };
      else if (event.detail === 2) start = { unit: "word", first: wordAt(pageText, spot) };
      else {
        const extending = event.shiftKey ? selection : null;
        start = { unit: "character", anchor: extending ? extending.anchor : caretAt(pageText, spot) };
      }
      setSelection(dragSelection(start, pageText, spot));
      startDrag(event.clientX, event.clientY, start);
    },
    onMouseMove(event) {
      if (drag.current) return;
      const found = textAt(event.clientX, event.clientY, true);
      const onText = found !== null && spotAt(found.pageText, found.at.page, found.at.point, TEXT_SLACK_PT) !== null;
      event.currentTarget.style.cursor = onText ? "text" : "";
    },
    hasSelection: () => hasSelectedText(latest.current.selection),
    async selectedText() {
      const current = latest.current.selection;
      const { source: text, doc: document } = latest.current.options;
      if (!hasSelectedText(current) || !text || document === undefined) return null;
      const [start, end] = selectionRange(current);
      const indexes = Array.from({ length: end.page - start.page + 1 }, (_, i) => start.page + i);
      const results = await Promise.allSettled(indexes.map((page) => text.textOnce(document, page)));
      const texts = new Map<number, PageText>();
      results.forEach((result, i) => {
        if (result.status === "fulfilled") texts.set(indexes[i]!, result.value);
      });
      return selectedText(current, (page) => texts.get(page));
    },
  };
}
