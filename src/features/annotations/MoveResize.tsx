// A chosen drawing or stamp can be dragged to move it, and resized by the handles on its corners
// and sides (B2-08, docs/architecture/annotations.md). While the pointer is down a dashed
// outline shows where it would go; the edit is sent when the pointer is released. A stamp keeps
// its shape (only its corners resize it); a drawing is resized freely, its line staying as thick.

import { useState, type PointerEvent } from "react";

import {
  dragged,
  HANDLES,
  isCorner,
  keptOnPage,
  moved,
  rectOfBox,
  type Box,
  type Handle,
} from "@/features/annotations/tools";
import type { PageSize, Rotation } from "@/features/shell/model";
import { displayedSize, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type PageAnnotation, type Rect } from "@/ipc/generated/contract";

type MoveResizeProps = {
  annotation: PageAnnotation;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  /** Where the annotation is on the page's box, in CSS pixels. */
  area: Box;
  /** The annotation is to be at `rect` (page space). */
  onPlace: (rect: Rect) => void;
};

type Drag = { handle: Handle | "move"; pointer: number; x: number; y: number };

/** Where the handle `handle` is on its box: fractions of its width and height. */
const AT: Record<Handle, [number, number]> = {
  nw: [0, 0],
  n: [0.5, 0],
  ne: [1, 0],
  e: [1, 0.5],
  se: [1, 1],
  s: [0.5, 1],
  sw: [0, 1],
  w: [0, 0.5],
};

const CURSOR: Record<Handle, string> = {
  nw: "nwse-resize",
  se: "nwse-resize",
  ne: "nesw-resize",
  sw: "nesw-resize",
  n: "ns-resize",
  s: "ns-resize",
  e: "ew-resize",
  w: "ew-resize",
};

/** The size of a handle, in CSS pixels. */
const HANDLE_PX = 10;

export function MoveResize({ annotation, page, rotation, box, area, onPlace }: MoveResizeProps) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [preview, setPreview] = useState<Box | null>(null);
  const keepShape = annotation.kind === "stamp";
  const pixelsPerPoint = box.width / displayedSize(page, rotation).widthPt;
  const least = LIMITS.minAnnotationSidePt * pixelsPerPoint;

  const start = (handle: Handle | "move") => (event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0 || drag) return;
    // No text is selected by this press, and nothing chosen by its click.
    event.preventDefault();
    event.stopPropagation();
    // Captured by the element that was pressed: its moves and its release come to the container.
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDrag({ handle, pointer: event.pointerId, x: event.clientX, y: event.clientY });
    setPreview(area);
  };
  const boxAt = (event: PointerEvent<HTMLElement>, from: Drag): Box => {
    const dx = event.clientX - from.x;
    const dy = event.clientY - from.y;
    return from.handle === "move" ? moved(area, dx, dy) : dragged(area, from.handle, dx, dy, least, keepShape);
  };
  const move = (event: PointerEvent<HTMLElement>) => {
    if (drag && event.pointerId === drag.pointer) setPreview(boxAt(event, drag));
  };
  const end = (event: PointerEvent<HTMLElement>) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    const where = boxAt(event, drag);
    setDrag(null);
    setPreview(null);
    const rect = keptOnPage(rectOfBox(where, page, rotation, box), page);
    const old = annotation.rect;
    const same = [rect.x0 - old.x0, rect.y0 - old.y0, rect.x1 - old.x1, rect.y1 - old.y1].every(
      (difference) => Math.abs(difference) < 0.01,
    );
    if (!same) onPlace(rect);
  };
  const cancel = (event: PointerEvent<HTMLElement>) => {
    if (drag && event.pointerId === drag.pointer) {
      setDrag(null);
      setPreview(null);
    }
  };

  return (
    <>
      <div
        data-move-resize={annotation.id}
        title={strings.annotations.moveHint}
        aria-hidden
        className="pointer-events-auto absolute cursor-move touch-none"
        style={{ left: area.left, top: area.top, width: area.width, height: area.height }}
        onPointerDown={start("move")}
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={cancel}
      >
        {HANDLES.filter((handle) => !keepShape || isCorner(handle)).map((handle) => (
          <div
            key={handle}
            data-handle={handle}
            className="absolute rounded-[2px] border border-primary bg-background"
            style={{
              left: `calc(${AT[handle][0] * 100}% - ${HANDLE_PX / 2}px)`,
              top: `calc(${AT[handle][1] * 100}% - ${HANDLE_PX / 2}px)`,
              width: HANDLE_PX,
              height: HANDLE_PX,
              cursor: CURSOR[handle],
            }}
            onPointerDown={start(handle)}
          />
        ))}
      </div>
      {preview && (
        <div
          data-move-preview
          className="pointer-events-none absolute border-2 border-dashed border-primary"
          style={{ left: preview.left, top: preview.top, width: preview.width, height: preview.height }}
        />
      )}
    </>
  );
}
