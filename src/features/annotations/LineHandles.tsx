// Line and arrow annotations can have their endpoints dragged to change length or orientation,
// or their middle dragged to move the entire line (B4-05).
// While dragging, a dashed line shows the preview; the edit is sent when released.

import { useState, type PointerEvent } from "react";

import type { PageSize, Rotation } from "@/features/shell/model";
import { boxToPage, pageToBox, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import type { PageAnnotation, Point } from "@/ipc/generated/contract";

type LineHandlesProps = {
  annotation: PageAnnotation;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  /** The endpoints are to be updated to `from` and `to` (page space). */
  onPlaceLine: (from: Point, to: Point) => void;
};

type Drag =
  | { handle: "from" | "to"; pointer: number; startX: number; startY: number }
  | { handle: "move"; pointer: number; startX: number; startY: number };

const HANDLE_PX = 10;

export function LineHandles({ annotation, page, rotation, box, onPlaceLine }: LineHandlesProps) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [preview, setPreview] = useState<{ from: { x: number; y: number }; to: { x: number; y: number } } | null>(
    null,
  );

  const initialFromPt: Point = annotation.lineEnds?.from ?? { x: annotation.rect.x0, y: annotation.rect.y0 };
  const initialToPt: Point = annotation.lineEnds?.to ?? { x: annotation.rect.x1, y: annotation.rect.y1 };

  const fromPx = pageToBox(initialFromPt, page, rotation, box);
  const toPx = pageToBox(initialToPt, page, rotation, box);
  const midPx = { x: (fromPx.x + toPx.x) / 2, y: (fromPx.y + toPx.y) / 2 };

  const start = (handle: "from" | "to" | "move") => (event: PointerEvent<Element>) => {
    if (event.button !== 0 || drag) return;
    event.preventDefault();
    event.stopPropagation();
    (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
    setDrag({ handle, pointer: event.pointerId, startX: event.clientX, startY: event.clientY });
    setPreview({ from: fromPx, to: toPx });
  };

  const move = (event: PointerEvent<Element>) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;

    if (drag.handle === "from") {
      setPreview({ from: { x: fromPx.x + dx, y: fromPx.y + dy }, to: toPx });
    } else if (drag.handle === "to") {
      setPreview({ from: fromPx, to: { x: toPx.x + dx, y: toPx.y + dy } });
    } else {
      setPreview({
        from: { x: fromPx.x + dx, y: fromPx.y + dy },
        to: { x: toPx.x + dx, y: toPx.y + dy },
      });
    }
  };

  const end = (event: PointerEvent<Element>) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    const current = preview ?? { from: fromPx, to: toPx };
    setDrag(null);
    setPreview(null);

    const newFrom = boxToPage(current.from, page, rotation, box);
    const newTo = boxToPage(current.to, page, rotation, box);

    // Enforce minimum line length of 2 points
    if (Math.hypot(newTo.x - newFrom.x, newTo.y - newFrom.y) < 2) return;

    const changed =
      Math.abs(newFrom.x - initialFromPt.x) > 0.01 ||
      Math.abs(newFrom.y - initialFromPt.y) > 0.01 ||
      Math.abs(newTo.x - initialToPt.x) > 0.01 ||
      Math.abs(newTo.y - initialToPt.y) > 0.01;

    if (changed) {
      onPlaceLine(newFrom, newTo);
    }
  };

  const cancel = (event: PointerEvent<Element>) => {
    if (drag && event.pointerId === drag.pointer) {
      setDrag(null);
      setPreview(null);
    }
  };

  const isArrow = annotation.kind === "arrow";

  return (
    <>
      <div
        data-line-handles={annotation.id}
        title={strings.annotations.moveLineHint}
        className="pointer-events-auto absolute inset-0 touch-none"
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={cancel}
      >
        {/* Invisible wider line to facilitate dragging the body */}
        <svg className="pointer-events-none absolute inset-0 size-full overflow-visible">
          <line
            x1={fromPx.x}
            y1={fromPx.y}
            x2={toPx.x}
            y2={toPx.y}
            stroke="transparent"
            strokeWidth={16}
            className="pointer-events-auto cursor-move"
            onPointerDown={start("move")}
          />
        </svg>

        {/* Start point handle */}
        <div
          data-handle="from"
          className="absolute rounded-full border border-primary bg-background shadow"
          style={{
            left: fromPx.x - HANDLE_PX / 2,
            top: fromPx.y - HANDLE_PX / 2,
            width: HANDLE_PX,
            height: HANDLE_PX,
            cursor: "crosshair",
          }}
          onPointerDown={start("from")}
        />

        {/* Middle move handle */}
        <div
          data-handle="mid"
          className="absolute rounded-[2px] border border-primary bg-background shadow"
          style={{
            left: midPx.x - HANDLE_PX / 2,
            top: midPx.y - HANDLE_PX / 2,
            width: HANDLE_PX,
            height: HANDLE_PX,
            cursor: "move",
          }}
          onPointerDown={start("move")}
        />

        {/* End point handle */}
        <div
          data-handle="to"
          className="absolute rounded-full border border-primary bg-background shadow"
          style={{
            left: toPx.x - HANDLE_PX / 2,
            top: toPx.y - HANDLE_PX / 2,
            width: HANDLE_PX,
            height: HANDLE_PX,
            cursor: "crosshair",
          }}
          onPointerDown={start("to")}
        />
      </div>

      {preview && (
        <svg className="pointer-events-none absolute inset-0 size-full overflow-visible">
          <defs>
            <marker
              id={`arrow-preview-${annotation.id}`}
              viewBox="0 0 10 10"
              refX="8"
              refY="5"
              markerWidth="6"
              markerHeight="6"
              orient="auto-start-reverse"
            >
              <path d="M 0 1.5 L 8 5 L 0 8.5 z" fill="currentColor" className="text-primary" />
            </marker>
          </defs>
          <line
            x1={preview.from.x}
            y1={preview.from.y}
            x2={preview.to.x}
            y2={preview.to.y}
            stroke="currentColor"
            strokeWidth={2}
            strokeDasharray="4 4"
            className="text-primary"
            markerEnd={isArrow ? `url(#arrow-preview-${annotation.id})` : undefined}
          />
        </svg>
      )}
    </>
  );
}
