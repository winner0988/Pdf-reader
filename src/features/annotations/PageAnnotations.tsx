// A page's annotations over it (B2-07, docs/architecture/annotations.md): MuPDF draws them on the
// page; here each gets a focusable outline, so that it can be chosen with the keyboard (or with
// a click, which the view finds by position: the outlines let the pointer through, so text under
// a highlighter mark can still be selected), and the chosen one a small toolbar. A chosen drawing
// or stamp can also be moved and resized (B2-08): by dragging, or with the arrow keys.

import { Pencil, Trash2 } from "lucide-react";
import { Fragment, useEffect, useState, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/button";
import { annotationLabel, HIGHLIGHT_COLORS, SWATCH } from "@/features/annotations/model";
import { MoveResize } from "@/features/annotations/MoveResize";
import { keptOnPage, moved, rectOfBox, type Box } from "@/features/annotations/tools";
import type { AnnotationSource } from "@/features/annotations/source";
import type { PageSize, Rotation } from "@/features/shell/model";
import { displayedSize, rectToBox, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import type { AnnotationId, DocumentId, Edit, HighlightColor, PageAnnotation, Rect } from "@/ipc/generated/contract";

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
};

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
}: PageAnnotationsProps) {
  const [loaded, setLoaded] = useState<{ doc: DocumentId; annotations: PageAnnotation[] } | null>(null);

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

  const annotations = loaded?.doc === doc ? loaded.annotations : [];
  const chosenOne = chosen === null ? null : (annotations.find((annotation) => annotation.id === chosen) ?? null);
  // An edit (a new document) can take the chosen annotation away.
  const gone = chosen !== null && loaded?.doc === doc && chosenOne === null;
  useEffect(() => {
    if (gone) onChoose(null);
  }, [gone, onChoose]);

  if (annotations.length === 0) return null;
  const remove = (annotation: PageAnnotation) =>
    onEdit?.({ kind: "deleteAnnotation", page: index, annotation: annotation.id });
  const place = (annotation: PageAnnotation, rect: Rect) =>
    onEdit?.({ kind: "setAnnotationRect", page: index, annotation: annotation.id, rect });
  const pixelsPerPoint = box.width / displayedSize(page, rotation).widthPt;
  const keys = (annotation: PageAnnotation, area: Box) => (event: KeyboardEvent) => {
    if ((event.key === "Delete" || event.key === "Backspace") && onEdit) {
      event.preventDefault();
      remove(annotation);
    } else if (event.key === "Escape") {
      event.preventDefault();
      onRelease();
    } else if (ARROWS[event.key] && onEdit && (annotation.kind === "ink" || annotation.kind === "stamp")) {
      // A drawing or a stamp moves a point at a time, ten with Shift, the way the arrows point.
      event.preventDefault();
      // Each press is an edit of its own, made on the document as the one before left it: a held
      // key would send them faster than the document is ready for the next.
      if (event.repeat) return;
      const [dx, dy] = ARROWS[event.key]!;
      const step = (event.shiftKey ? 10 : 1) * pixelsPerPoint;
      place(annotation, keptOnPage(rectOfBox(moved(area, dx * step, dy * step), page, rotation, box), page));
    }
  };
  return (
    <div
      data-page-annotations={index + 1}
      className="pointer-events-none absolute"
      style={{ top: box.top, left, width: box.width, height: box.height }}
    >
      {annotations.map((annotation) => {
        const area = rectToBox(annotation.rect, page, rotation, box);
        const isChosen = annotation.id === chosen;
        const movable = onEdit !== undefined && (annotation.kind === "ink" || annotation.kind === "stamp");
        return (
          // The chosen one's toolbar follows its outline, so that Tab goes from one to the other.
          <Fragment key={annotation.id}>
            <button
              type="button"
              aria-label={annotationLabel(annotation)}
              aria-pressed={isChosen}
              aria-description={movable ? strings.annotations.moveHint : undefined}
              data-annotation={annotation.kind}
              // The pointer goes through: a click is found by position (DocumentView).
              className="absolute rounded-[2px] outline-offset-2 focus-visible:outline-2 focus-visible:outline-primary aria-pressed:outline-2 aria-pressed:outline-primary aria-pressed:outline-dashed"
              style={{ left: area.left, top: area.top, width: area.width, height: area.height }}
              onFocus={() => onChoose(annotation.id)}
              onKeyDown={keys(annotation, area)}
            />
            {isChosen && movable && (
              <MoveResize
                annotation={annotation}
                page={page}
                rotation={rotation}
                box={box}
                area={area}
                onPlace={(rect) => place(annotation, rect)}
              />
            )}
            {isChosen && (
              <AnnotationToolbar
                annotation={annotation}
                // Above it, unless there is no room on the page.
                top={area.top >= 40 ? area.top - 40 : area.top + area.height + 4}
                left={Math.max(0, Math.min(area.left, box.width - 240))}
                allowed={onEdit !== undefined}
                onColor={(color) =>
                  onEdit?.({ kind: "setHighlightColor", page: index, annotation: annotation.id, color })
                }
                onEditNote={() => onEditNote?.(index, annotation)}
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
  onEditNote: () => void;
  onDelete: () => void;
  onKeyDown: (event: KeyboardEvent) => void;
};

function AnnotationToolbar({
  annotation,
  top,
  left,
  allowed,
  onColor,
  onEditNote,
  onDelete,
  onKeyDown,
}: AnnotationToolbarProps) {
  const why = allowed ? undefined : t.notAllowed;
  return (
    <div
      role="toolbar"
      aria-label={annotationLabel(annotation)}
      className="pointer-events-auto absolute flex max-w-[240px] items-center gap-1 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
      style={{ top, left }}
      // A press here is not a press on the page: no text selection starts, nothing is chosen.
      onMouseDown={(event) => event.stopPropagation()}
      onClick={(event) => event.stopPropagation()}
      onKeyDown={onKeyDown}
    >
      {annotation.kind === "highlight" &&
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
      {(annotation.kind === "ink" || annotation.kind === "stamp" || annotation.kind === "other") && (
        <span className="px-1">{t.kind[annotation.kind]}</span>
      )}
      <Button variant="ghost" size="icon-sm" aria-label={t.delete} title={why ?? t.delete} disabled={!allowed} onClick={onDelete}>
        <Trash2 />
      </Button>
    </div>
  );
}
