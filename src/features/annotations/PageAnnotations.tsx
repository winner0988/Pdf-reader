// A page's annotations over it (B2-07, docs/architecture/annotations.md): MuPDF draws them on the
// page; here each gets a focusable outline, so that it can be chosen with the keyboard (or with
// a click, which the view finds by position: the outlines let the pointer through, so text under
// a highlighter mark can still be selected), and the chosen one a small toolbar. A chosen drawing
// or stamp can also be moved and resized (B2-08): by dragging, or with the arrow keys; a picture
// of the user's own can be turned too (B3-01): by dragging its handle, with the buttons in its
// toolbar, or with the keys [ and ]; so can a text box (B2-30), which also has Enter and a button
// to change what it says.

import { ChevronDown, Pencil, RotateCcw, RotateCw, Trash2 } from "lucide-react";
import { Fragment, useEffect, useRef, useState, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { LineHandles } from "@/features/annotations/LineHandles";
import { annotationLabel, HIGHLIGHT_COLORS, SWATCH } from "@/features/annotations/model";
import { MoveResize } from "@/features/annotations/MoveResize";
import { PictureTurn } from "@/features/annotations/PictureTurn";
import { normalizeAngle, TURN_STEP_DEGREES } from "@/features/annotations/turn";
import { INK_COLORS, INK_SWATCH, INK_WIDTHS, keepLineOnPage, keptOnPage, moved, rectOfBox, type Box } from "@/features/annotations/tools";
import type { AnnotationSource } from "@/features/annotations/source";
import type { PageSize, Rotation } from "@/features/shell/model";
import { boxToPage, displayedSize, pageToBox, rectToBox, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import type {
  AnnotationId,
  DocumentId,
  Edit,
  HighlightColor,
  InkColor,
  InkWidth,
  PageAnnotation,
  Point,
  Rect,
} from "@/ipc/generated/contract";

const t = strings.annotations;

/** What each arrow key moves a drawing or stamp: along x and along y, on the screen. */
const ARROWS: Record<string, [number, number] | undefined> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};

type PageAnnotationsProps = {
  source: AnnotationSource;
  doc: DocumentId;
  index: number;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  left: number;
  delayMs: number;
  /** The chosen annotation, when it is on this page. */
  chosen: AnnotationId | null;
  onChoose: (annotation: AnnotationId | null) => void;
  /** Lets go of the chosen annotation and gives the focus back to the view (Esc). */
  onRelease: () => void;
  /** Changes an annotation; without it (the author does not allow it) they can only be looked at. */
  onEdit?: (edit: Edit) => void;
  /** Asks for a note's new text (the shell's dialog). */
  onEditNote?: (page: number, annotation: PageAnnotation) => void;
  /** Asks for a text box's new words and style (the shell's dialog); without it the box can only be moved or removed. */
  onEditTextBox?: (page: number, annotation: PageAnnotation) => void;
};

/** The kinds that can be moved and resized; a redaction mark is only an area until it is applied (B2-19). */
const MOVABLE = new Set(["ink", "stamp", "textBox", "redaction", "rectangle", "ellipse", "line", "arrow"]);

export function PageAnnotations({
  source,
  doc,
  index,
  page,
  rotation,
  box,
  left,
  delayMs,
  chosen,
  onChoose,
  onRelease,
  onEdit,
  onEditNote,
  onEditTextBox,
}: PageAnnotationsProps) {
  // The annotations of the last document that answered. An edit gives the document a new id; the
  // annotation the user is on keeps its outline, and with it the focus, until the new list arrives
  // (#402).
  const [loaded, setLoaded] = useState<{ doc: DocumentId; annotations: PageAnnotation[] } | null>(null);
  // The focus is on an outline or a toolbar of this page. It has left only when something else gets
  // it: one taken off the page takes the focus to nowhere (WebView2 blurs it as it goes, with no
  // element to go to), and so does the window losing it, and this stays true then.
  const focusInside = useRef(false);

  useEffect(() => {
    let current = true;
    const load = () =>
      source.annotations(doc, index).then(
        (annotations) => {
          if (current) setLoaded({ doc, annotations });
        },
        // No outlines is better than an error: the page still shows them.
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

  // Whether the list is of the document the page shows, and not of the one before an edit.
  const fresh = loaded?.doc === doc;
  const annotations = loaded?.annotations ?? [];
  const chosenOne = chosen === null ? null : (annotations.find((annotation) => annotation.id === chosen) ?? null);
  // An edit (a new document) can take the chosen annotation away.
  const gone = chosen !== null && fresh && chosenOne === null;
  useEffect(() => {
    if (!gone) return;
    if (focusInside.current && (document.activeElement === null || document.activeElement === document.body)) {
      // The focus went with it (Delete, say): the view gets it back, as with Esc.
      focusInside.current = false;
      onRelease();
    } else onChoose(null);
  }, [gone, onChoose, onRelease]);

  // When an annotation is chosen (e.g. from the sidebar), focus its outline once the page's
  // annotations are loaded (B4-06).
  const chosenBtnRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    if (chosen !== null && chosenBtnRef.current && document.activeElement !== chosenBtnRef.current) {
      if (!document.activeElement?.closest('[role="dialog"]')) {
        chosenBtnRef.current.focus();
      }
    }
  }, [chosen, loaded]);

  if (annotations.length === 0) return null;
  // Nothing is changed from the list of the document before an edit, and no dialog starts from it:
  // they wait for the new list.
  const change = (edit: Edit) => {
    if (fresh) onEdit?.(edit);
  };
  const remove = (annotation: PageAnnotation) =>
    change({ kind: "deleteAnnotation", page: index, annotation: annotation.id });
  const place = (annotation: PageAnnotation, rect: Rect) =>
    change({ kind: "setAnnotationRect", page: index, annotation: annotation.id, rect });
  const placeLine = (annotation: PageAnnotation, from: Point, to: Point) =>
    change({ kind: "setLineEnds", page: index, annotation: annotation.id, from, to });
  const setShapeStyle = (annotation: PageAnnotation, color: InkColor, width: InkWidth) =>
    change({ kind: "setShapeStyle", page: index, annotation: annotation.id, color, width });
  // A picture is turned to an angle (B3-01), not by a step on top of what the page last heard.
  const turnTo = (annotation: PageAnnotation, degrees: number) =>
    change({ kind: "setPictureAngle", page: index, annotation: annotation.id, degrees: normalizeAngle(degrees) });
  const editNote = (annotation: PageAnnotation) => {
    if (fresh) onEditNote?.(index, annotation);
  };
  const editTextBox =
    onEditTextBox &&
    ((annotation: PageAnnotation) => {
      if (fresh) onEditTextBox(index, annotation);
    });
  const pixelsPerPoint = box.width / displayedSize(page, rotation).widthPt;
  const keys = (annotation: PageAnnotation, area: Box) => (event: KeyboardEvent) => {
    if ((event.key === "Delete" || event.key === "Backspace") && onEdit) {
      event.preventDefault();
      remove(annotation);
    } else if (event.key === "Escape") {
      event.preventDefault();
      onRelease();
    } else if (event.key === "Enter" && annotation.kind === "textBox" && onEdit && editTextBox) {
      // Enter changes what a text box says: the keyboard's way to the dialog.
      event.preventDefault();
      if (!event.repeat) editTextBox(annotation);
    } else if ((event.key === "[" || event.key === "]") && onEdit && annotation.picture) {
      // A picture is turned a step at a time, the way the bracket points (B3-01).
      event.preventDefault();
      if (event.repeat) return;
      const by = event.key === "]" ? TURN_STEP_DEGREES : -TURN_STEP_DEGREES;
      turnTo(annotation, annotation.picture.angle + by);
    } else if (ARROWS[event.key] && onEdit && MOVABLE.has(annotation.kind)) {
      // A drawing, a stamp, a shape or a text box moves a point at a time, ten with Shift, the way the arrows point.
      event.preventDefault();
      // Each press is an edit of its own, made on the document as the one before left it: a held
      // key would send them faster than the document is ready for the next.
      if (event.repeat) return;
      const [dx, dy] = ARROWS[event.key]!;
      const step = (event.shiftKey ? 10 : 1) * pixelsPerPoint;
      if (annotation.kind === "line" || annotation.kind === "arrow") {
        const fromPt: Point = annotation.lineEnds?.from ?? { x: annotation.rect.x0, y: annotation.rect.y0 };
        const toPt: Point = annotation.lineEnds?.to ?? { x: annotation.rect.x1, y: annotation.rect.y1 };
        const p1 = pageToBox(fromPt, page, rotation, box);
        const p2 = pageToBox(toPt, page, rotation, box);
        const nextFrom = boxToPage({ x: p1.x + dx * step, y: p1.y + dy * step }, page, rotation, box);
        const nextTo = boxToPage({ x: p2.x + dx * step, y: p2.y + dy * step }, page, rotation, box);
        const kept = keepLineOnPage(nextFrom, nextTo, page);
        placeLine(annotation, kept.from, kept.to);
      } else {
        place(annotation, keptOnPage(rectOfBox(moved(area, dx * step, dy * step), page, rotation, box), page));
      }
    }
  };
  return (
    <div
      data-page-annotations={index + 1}
      // An edit was just made: these are the annotations before it, until the new list arrives.
      aria-busy={fresh ? undefined : true}
      className="pointer-events-none absolute"
      style={{ top: box.top, left, width: box.width, height: box.height }}
      onFocus={() => {
        focusInside.current = true;
      }}
      onBlur={(event) => {
        const to = event.relatedTarget;
        if (to instanceof Node && !event.currentTarget.contains(to)) focusInside.current = false;
      }}
    >
      {annotations.map((annotation) => {
        const area = rectToBox(annotation.rect, page, rotation, box);
        const isChosen = annotation.id === chosen;
        const movable = onEdit !== undefined && MOVABLE.has(annotation.kind);
        const isLine = annotation.kind === "line" || annotation.kind === "arrow";
        const btnWidth = isLine ? Math.max(area.width, 16) : area.width;
        const btnHeight = isLine ? Math.max(area.height, 16) : area.height;
        const btnLeft = isLine && area.width < 16 ? area.left - (16 - area.width) / 2 : area.left;
        const btnTop = isLine && area.height < 16 ? area.top - (16 - area.height) / 2 : area.top;
        return (
          // The chosen one's toolbar follows its outline, so that Tab goes from one to the other.
          <Fragment key={annotation.id}>
            <button
              ref={isChosen ? chosenBtnRef : undefined}
              type="button"
              aria-label={annotationLabel(annotation)}
              aria-pressed={isChosen}
              aria-description={
                movable
                  ? annotation.picture
                    ? t.moveTurnHint
                    : isLine
                    ? t.moveLineHint
                    : t.moveHint
                  : undefined
              }
              data-annotation={annotation.kind}
              title={annotation.kind === "redaction" ? t.kind.redaction : undefined}
              // The pointer goes through: a click is found by position (DocumentView).
              className={`absolute rounded-[2px] outline-offset-2 focus-visible:outline-2 focus-visible:outline-primary aria-pressed:outline-2 aria-pressed:outline-primary aria-pressed:outline-dashed ${
                annotation.kind === "redaction" ? "border border-red-500/80 bg-red-500/15" : ""
              }`}
              style={{ left: btnLeft, top: btnTop, width: btnWidth, height: btnHeight }}
              onFocus={() => {
                if (chosen !== annotation.id) {
                  onChoose(annotation.id);
                }
              }}
              onKeyDown={keys(annotation, area)}
            />
            {/* The handles are for the annotation as the page shows it: not on the list before an edit. */}
            {isChosen && movable && fresh && isLine && (
              <LineHandles
                annotation={annotation}
                page={page}
                rotation={rotation}
                box={box}
                onPlaceLine={(from, to) => placeLine(annotation, from, to)}
              />
            )}
            {isChosen && movable && fresh && !isLine && (
              <MoveResize
                annotation={annotation}
                page={page}
                rotation={rotation}
                box={box}
                area={area}
                onPlace={(rect) => place(annotation, rect)}
              />
            )}
            {isChosen && movable && fresh && annotation.picture && (
              <PictureTurn
                picture={annotation.picture}
                rect={annotation.rect}
                page={page}
                rotation={rotation}
                box={box}
                onTurn={(degrees) => turnTo(annotation, degrees)}
              />
            )}
            {isChosen && (
              <AnnotationToolbar
                annotation={annotation}
                // Above it (and above the handle that turns a picture), unless there is no room on the page.
                top={
                  area.top >= (annotation.picture ? 72 : 40)
                    ? area.top - (annotation.picture ? 72 : 40)
                    : area.top + area.height + 4
                }
                left={Math.max(0, Math.min(area.left, box.width - 240))}
                allowed={onEdit !== undefined}
                onColor={(color) =>
                  change({ kind: "setHighlightColor", page: index, annotation: annotation.id, color })
                }
                onShapeColor={(color) =>
                  setShapeStyle(annotation, color, annotation.shapeWidth ?? "medium")
                }
                onShapeWidth={(width) =>
                  setShapeStyle(annotation, annotation.shapeColor ?? "black", width)
                }
                onEditNote={() => editNote(annotation)}
                onEditTextBox={editTextBox ? () => editTextBox(annotation) : undefined}
                onTurn={
                  annotation.picture
                    ? (by) => turnTo(annotation, (annotation.picture?.angle ?? 0) + by)
                    : undefined
                }
                onDelete={() => remove(annotation)}
                onKeyDown={keys(annotation, area)}
              />
            )}
          </Fragment>
        );
      })}
    </div>
  );
}

type AnnotationToolbarProps = {
  annotation: PageAnnotation;
  top: number;
  left: number;
  allowed: boolean;
  onColor: (color: HighlightColor) => void;
  onShapeColor?: (color: InkColor) => void;
  onShapeWidth?: (width: InkWidth) => void;
  onEditNote: () => void;
  onEditTextBox?: () => void;
  /** Turns the picture by so many degrees (B3-01); given only for a picture stamp. */
  onTurn?: (by: number) => void;
  onDelete: () => void;
  onKeyDown: (event: KeyboardEvent) => void;
};

function AnnotationToolbar({
  annotation,
  top,
  left,
  allowed,
  onColor,
  onShapeColor,
  onShapeWidth,
  onEditNote,
  onEditTextBox,
  onTurn,
  onDelete,
  onKeyDown,
}: AnnotationToolbarProps) {
  const why = allowed ? undefined : t.notAllowed;
  const isShape =
    annotation.kind === "rectangle" ||
    annotation.kind === "ellipse" ||
    annotation.kind === "line" ||
    annotation.kind === "arrow";
  return (
    <div
      role="toolbar"
      aria-label={annotationLabel(annotation)}
      className="pointer-events-auto absolute flex max-w-[260px] items-center gap-1 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
      style={{ top, left }}
      // A press here is not a press on the page: no text selection starts, nothing is chosen.
      onMouseDown={(event) => event.stopPropagation()}
      onClick={(event) => event.stopPropagation()}
      onKeyDown={onKeyDown}
    >
      {(annotation.kind === "highlight" ||
        annotation.kind === "underline" ||
        annotation.kind === "strikeOut" ||
        annotation.kind === "squiggly") &&
        HIGHLIGHT_COLORS.map((color) => (
          <button
            key={color}
            type="button"
            aria-label={t.colors[color]}
            aria-pressed={annotation.color === color}
            title={why ?? t.colors[color]}
            disabled={!allowed}
            className="size-6 rounded-full border border-black/20 outline-offset-1 focus-visible:outline-2 focus-visible:outline-primary disabled:opacity-50 aria-pressed:ring-2 aria-pressed:ring-primary"
            style={{ backgroundColor: SWATCH[color] }}
            onClick={() => onColor(color)}
          />
        ))}
      {isShape && (
        <>
          {INK_COLORS.map((color) => (
            <button
              key={color}
              type="button"
              aria-label={t.inkColors[color]}
              aria-pressed={annotation.shapeColor === color}
              title={why ?? t.inkColors[color]}
              disabled={!allowed}
              className="size-6 rounded-full border border-black/20 outline-offset-1 focus-visible:outline-2 focus-visible:outline-primary disabled:opacity-50 aria-pressed:ring-2 aria-pressed:ring-primary"
              style={{ backgroundColor: INK_SWATCH[color] }}
              onClick={() => onShapeColor?.(color)}
            />
          ))}
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t.shapeStyle}
                  title={why ?? t.shapeStyle}
                  disabled={!allowed}
                >
                  <ChevronDown />
                </Button>
              }
            />
            <DropdownMenuContent align="end" className="w-32">
              <DropdownMenuGroup>
                <DropdownMenuRadioGroup
                  value={annotation.shapeWidth ?? "medium"}
                  onValueChange={(val) => onShapeWidth?.(val as InkWidth)}
                >
                  {INK_WIDTHS.map((width) => (
                    <DropdownMenuRadioItem key={width} value={width}>
                      {t.inkWidths[width]}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      )}
      {annotation.kind === "note" && (
        <>
          <span className="min-w-0 truncate px-1" title={annotation.text ?? undefined}>
            {annotation.text ?? t.kind.note}
          </span>
          <Button variant="ghost" size="icon-sm" aria-label={t.editNote} title={why ?? t.editNote} disabled={!allowed} onClick={onEditNote}>
            <Pencil />
          </Button>
        </>
      )}
      {annotation.kind === "textBox" && (
        <>
          <span className="min-w-0 truncate px-1" title={annotation.text ?? undefined}>
            {annotation.text ?? t.kind.textBox}
          </span>
          {onEditTextBox && (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t.editTextBox}
              title={why ?? t.editTextBox}
              disabled={!allowed}
              onClick={onEditTextBox}
            >
              <Pencil />
            </Button>
          )}
        </>
      )}
      {(annotation.kind === "ink" ||
        annotation.kind === "stamp" ||
        annotation.kind === "redaction" ||
        annotation.kind === "other") && (
        <span className="px-1">{t.kind[annotation.kind]}</span>
      )}
      {onTurn && (
        <>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t.turnLeft}
            title={why ?? t.turnLeft}
            disabled={!allowed}
            onClick={() => onTurn(-TURN_STEP_DEGREES)}
          >
            <RotateCcw />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t.turnRight}
            title={why ?? t.turnRight}
            disabled={!allowed}
            onClick={() => onTurn(TURN_STEP_DEGREES)}
          >
            <RotateCw />
          </Button>
        </>
      )}
      <Button variant="ghost" size="icon-sm" aria-label={t.delete} title={why ?? t.delete} disabled={!allowed} onClick={onDelete}>
        <Trash2 />
      </Button>
    </div>
  );
}
