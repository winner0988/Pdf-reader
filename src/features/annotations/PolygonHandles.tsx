// Polygon and polyline annotations allow dragging individual vertices (`SetShapeVertices`) or
// dragging the whole shape (`SetAnnotationRect`) with a dashed SVG preview (B7-10, #745).

import { useState, type PointerEvent } from "react";

import {
  keepVerticesOnPage,
  keptPointOnPage,
  polygonMoveRect,
  translateVertices,
} from "@/features/annotations/tools";
import type { PageSize, Rotation } from "@/features/shell/model";
import { boxToPage, pageToBox, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n";
import type { PageAnnotation, Point, Rect } from "@/ipc/generated/contract";

type PolygonHandlesProps = {
  annotation: PageAnnotation;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  /** Emits updated vertices after dragging a single vertex handle (page space). */
  onVertices: (vertices: Point[]) => void;
  /** Emits a translated bounding rect when moving the whole polygon/polyline. */
  onMove: (rect: Rect) => void;
};

type Drag =
  | { kind: "vertex"; index: number; pointer: number; startClient: Point; origPx: Point[] }
  | { kind: "move"; pointer: number; startClient: Point; origPx: Point[] };

const HANDLE_PX = 10;

export function PolygonHandles({
  annotation,
  page,
  rotation,
  box,
  onVertices,
  onMove,
}: PolygonHandlesProps) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const [previewPx, setPreviewPx] = useState<Point[] | null>(null);

  const vertices = annotation.vertices ?? [];
  if (vertices.length < 2) return null;

  const isClosed = annotation.kind === "polygon";
  const basePx = vertices.map((p) => pageToBox(p, page, rotation, box));

  const startVertex = (index: number) => (event: PointerEvent<Element>) => {
    if (event.button !== 0 || drag) return;
    event.preventDefault();
    event.stopPropagation();
    (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
    setDrag({
      kind: "vertex",
      index,
      pointer: event.pointerId,
      startClient: { x: event.clientX, y: event.clientY },
      origPx: basePx,
    });
    setPreviewPx(basePx);
  };

  const startMove = (event: PointerEvent<Element>) => {
    if (event.button !== 0 || drag) return;
    event.preventDefault();
    event.stopPropagation();
    (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
    setDrag({
      kind: "move",
      pointer: event.pointerId,
      startClient: { x: event.clientX, y: event.clientY },
      origPx: basePx,
    });
    setPreviewPx(basePx);
  };

  const move = (event: PointerEvent<Element>) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    const dx = event.clientX - drag.startClient.x;
    const dy = event.clientY - drag.startClient.y;
    if (drag.kind === "vertex") {
      setPreviewPx(
        drag.origPx.map((p, i) => (i === drag.index ? { x: p.x + dx, y: p.y + dy } : p)),
      );
    } else {
      setPreviewPx(drag.origPx.map((p) => ({ x: p.x + dx, y: p.y + dy })));
    }
  };

  const end = (event: PointerEvent<Element>) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    const dxPx = event.clientX - drag.startClient.x;
    const dyPx = event.clientY - drag.startClient.y;
    const activeDrag = drag;
    const finalPx =
      previewPx ??
      (activeDrag.kind === "vertex"
        ? activeDrag.origPx.map((p, i) =>
            i === activeDrag.index ? { x: p.x + dxPx, y: p.y + dyPx } : p,
          )
        : activeDrag.origPx.map((p) => ({ x: p.x + dxPx, y: p.y + dyPx })));
    setDrag(null);
    setPreviewPx(null);

    if (activeDrag.kind === "vertex") {
      const movedPt = keptPointOnPage(
        boxToPage(finalPx[activeDrag.index]!, page, rotation, box),
        page,
      );
      const nextVertices = vertices.map((p, i) => (i === activeDrag.index ? movedPt : p));
      // Forbid adjacent identical points and no-op drags.
      const prev = nextVertices[activeDrag.index - 1];
      const next = nextVertices[activeDrag.index + 1];
      const adjacentSame =
        (prev && Math.hypot(prev.x - movedPt.x, prev.y - movedPt.y) < 1e-3) ||
        (next && Math.hypot(next.x - movedPt.x, next.y - movedPt.y) < 1e-3);
      if (adjacentSame) return;

      const orig = vertices[activeDrag.index]!;
      if (Math.hypot(movedPt.x - orig.x, movedPt.y - orig.y) > 0.01) {
        onVertices(nextVertices);
      }
    } else {
      const originPage = boxToPage(activeDrag.origPx[0]!, page, rotation, box);
      const movedPage = boxToPage(finalPx[0]!, page, rotation, box);
      const clampedDelta = keepVerticesOnPage(
        vertices,
        movedPage.x - originPage.x,
        movedPage.y - originPage.y,
        page,
      );
      if (Math.hypot(clampedDelta.dx, clampedDelta.dy) > 0.01) {
        const translated = translateVertices(vertices, clampedDelta.dx, clampedDelta.dy);
        if (translated.length >= 2) {
          onMove(polygonMoveRect(annotation.rect, clampedDelta.dx, clampedDelta.dy));
        }
      }
    }
  };

  const cancel = (event: PointerEvent<Element>) => {
    if (drag && event.pointerId === drag.pointer) {
      setDrag(null);
      setPreviewPx(null);
    }
  };

  const svgPoints = (previewPx ?? basePx).map((p) => `${p.x},${p.y}`).join(" ");

  return (
    <>
      <div
        data-polygon-handles={annotation.id}
        title={strings.annotations.movePolygonHint}
        className="pointer-events-auto absolute inset-0 touch-none"
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={cancel}
      >
        <svg className="pointer-events-none absolute inset-0 size-full overflow-visible">
          {isClosed ? (
            <polygon
              points={svgPoints}
              fill="transparent"
              stroke="transparent"
              strokeWidth={14}
              className="pointer-events-auto cursor-move"
              onPointerDown={startMove}
            />
          ) : (
            <polyline
              points={svgPoints}
              fill="none"
              stroke="transparent"
              strokeWidth={16}
              className="pointer-events-auto cursor-move"
              onPointerDown={startMove}
            />
          )}
        </svg>

        {basePx.map((pt, i) => (
          <div
            key={i}
            data-vertex-index={i}
            className="absolute rounded-full border border-primary bg-background shadow"
            style={{
              left: pt.x - HANDLE_PX / 2,
              top: pt.y - HANDLE_PX / 2,
              width: HANDLE_PX,
              height: HANDLE_PX,
              cursor: "crosshair",
            }}
            onPointerDown={startVertex(i)}
          />
        ))}
      </div>

      {previewPx && (
        <svg className="pointer-events-none absolute inset-0 size-full overflow-visible">
          {isClosed ? (
            <polygon
              points={svgPoints}
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
              strokeDasharray="4 4"
              className="text-primary"
            />
          ) : (
            <polyline
              points={svgPoints}
              fill="none"
              stroke="currentColor"
              strokeWidth={2}
              strokeDasharray="4 4"
              className="text-primary"
            />
          )}
        </svg>
      )}
    </>
  );
}
