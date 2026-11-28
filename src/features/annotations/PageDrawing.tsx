// The layer over a page that takes the pointer while a tool is chosen (B2-08,
// docs/architecture/annotations.md): the pen draws a stroke, which is shown as it is drawn and
// sent when the pointer is released; a stamp is put down where the pointer is released. The layer
// is above the page's other layers, so that nothing under it (text, links, fields) gets the press.

import { useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";

import {
  INK_SWATCH,
  INK_WIDTH_PT,
  keptOnPage,
  keptPointOnPage,
  rectOfBox,
  shapeLine,
  shapeRect,
  snapSegmentAngle,
  strokePoints,
  type Tool,
} from "@/features/annotations/tools";
import type { PageSize, Rotation } from "@/features/shell/model";
import { boxToPage, displayedSize, type PageBox } from "@/features/viewer/layout";
import { rectOfPoints } from "@/features/redaction/model";
import type { InkColor, InkWidth, Point, Rect, Shape } from "@/ipc/generated/contract";

type PageDrawingProps = {
  tool: Tool;
  index: number;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  left: number;
  /** A stroke of the pen is done: its points on the page (page space), thinned. */
  onStroke: (index: number, points: Point[]) => void;
  /** The pointer was released on the page, to put a stamp down there (page space). */
  onPlace: (index: number, at: Point) => void;
  /** An arbitrary rectangular area was dragged on the page (for redaction, B2-21). */
  onRect?: (index: number, rect: Rect) => void;
  /** A shape annotation was drawn on the page (B4-05). */
  onShape?: (index: number, shape: Shape, color: InkColor, width: InkWidth) => void;
};

export function PageDrawing({
  tool,
  index,
  page,
  rotation,
  box,
  left,
  onStroke,
  onPlace,
  onRect,
  onShape,
}: PageDrawingProps) {
  const layer = useRef<HTMLDivElement>(null);
  /** The pointer that is drawing, and where it has been, in CSS pixels of the page's box. */
  const pointer = useRef<number | null>(null);
  const trail = useRef<Point[]>([]);
  /** What is shown of it: the trail and shift modifier, replaced (not changed) with every move. */
  const [shown, setShown] = useState<{ points: Point[]; shift: boolean }>({ points: [], shift: false });
  const follow = (points: Point[], shift: boolean) => {
    trail.current = points;
    setShown({ points, shift });
  };

  /** Multi-vertex state (in CSS pixels) for polygon and polyLine drawing (B7-10). */
  const isPolyTool =
    tool.kind === "shape" && (tool.shapeKind === "polygon" || tool.shapeKind === "polyLine");
  const polyRef = useRef<Point[]>([]);
  const [poly, setPoly] = useState<{ vertices: Point[]; hover: Point | null; shift: boolean }>({
    vertices: [],
    hover: null,
    shift: false,
  });
  const resetPoly = () => {
    polyRef.current = [];
    setPoly({ vertices: [], hover: null, shift: false });
  };

  const inBox = (event: PointerEvent): Point => {
    const bounds = layer.current?.getBoundingClientRect();
    return { x: event.clientX - (bounds?.left ?? 0), y: event.clientY - (bounds?.top ?? 0) };
  };
  // The pointer can leave the page while it is down: what is drawn or put down stays on the page.
  const onPage = (point: Point) => keptPointOnPage(boxToPage(point, page, rotation, box), page);
  const roundPt = (point: Point): Point => ({
    x: Math.round(point.x * 100) / 100,
    y: Math.round(point.y * 100) / 100,
  });
  const samePoint = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y) < 0.5;

  const commitPoly = () => {
    if (!isPolyTool || tool.kind !== "shape" || !onShape) {
      resetPoly();
      return;
    }
    const pageVerts: Point[] = [];
    for (const pt of polyRef.current) {
      const p = roundPt(onPage(pt));
      const last = pageVerts.at(-1);
      if (!last || last.x !== p.x || last.y !== p.y) pageVerts.push(p);
    }
    const min = tool.shapeKind === "polygon" ? 3 : 2;
    resetPoly();
    if (pageVerts.length < min) return;
    const shape: Shape =
      tool.shapeKind === "polygon"
        ? { kind: "polygon", vertices: pageVerts, cloudy: tool.cloudy }
        : { kind: "polyLine", vertices: pageVerts };
    onShape(index, shape, tool.color, tool.width);
  };

  useEffect(() => {
    if (!isPolyTool || poly.vertices.length === 0) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        resetPoly();
      } else if (event.key === "Enter" && !event.repeat) {
        const target = event.target;
        const typing =
          target instanceof Element && target.closest("input, textarea, select") !== null;
        if (typing) return;
        event.preventDefault();
        event.stopPropagation();
        commitPoly();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  });

  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || pointer.current !== null) return;
    // No text is selected by this press, and no annotation chosen by its click.
    event.preventDefault();
    if (isPolyTool) {
      pointer.current = event.pointerId;
      return;
    }
    pointer.current = event.pointerId;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    follow([inBox(event)], event.shiftKey);
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (isPolyTool) {
      if (polyRef.current.length > 0) {
        const pt = inBox(event);
        setPoly((prev) => ({ ...prev, hover: pt, shift: event.shiftKey }));
      }
      return;
    }
    if (event.pointerId !== pointer.current) return;
    follow([...trail.current, inBox(event)], event.shiftKey);
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    if (isPolyTool) {
      if (event.pointerId !== pointer.current) return;
      pointer.current = null;
      let pt = inBox(event);
      const anchor = polyRef.current.at(-1);
      if (anchor && event.shiftKey) {
        pt = snapSegmentAngle(anchor, pt);
      }
      if (!anchor || !samePoint(anchor, pt)) {
        const next = [...polyRef.current, pt];
        polyRef.current = next;
        setPoly({ vertices: next, hover: pt, shift: event.shiftKey });
      }
      return;
    }
    if (event.pointerId !== pointer.current) return;
    pointer.current = null;
    const end = inBox(event);
    const points = [...trail.current, end];
    const start = trail.current[0];
    follow([], false);
    if (tool.kind === "pen") {
      onStroke(index, strokePoints(points.map(onPage)));
    } else if (tool.kind === "redaction" || tool.kind === "link") {
      if (start && Math.hypot(end.x - start.x, end.y - start.y) >= 4 && onRect) {
        onRect(index, rectOfPoints(onPage(start), onPage(end)));
      } else {
        onPlace(index, onPage(end));
      }
    } else if (tool.kind === "shape") {
      if (start && onShape) {
        if (tool.shapeKind === "rectangle" || tool.shapeKind === "ellipse") {
          const r = shapeRect(start, end, event.shiftKey);
          const pageRect = keptOnPage(
            rectOfBox({ left: r.x0, top: r.y0, width: r.x1 - r.x0, height: r.y1 - r.y0 }, page, rotation, box),
            page,
          );
          if (Math.max(pageRect.x1 - pageRect.x0, pageRect.y1 - pageRect.y0) >= 2) {
            const shape: Shape =
              tool.shapeKind === "rectangle"
                ? { kind: "rectangle", rect: pageRect, cloudy: tool.cloudy }
                : { kind: "ellipse", rect: pageRect };
            onShape(index, shape, tool.color, tool.width);
          }
        } else {
          const l = shapeLine(start, end, event.shiftKey);
          const from = keptPointOnPage(boxToPage(l.from, page, rotation, box), page);
          const to = keptPointOnPage(boxToPage(l.to, page, rotation, box), page);
          if (Math.hypot(to.x - from.x, to.y - from.y) >= 2) {
            const shape: Shape = {
              kind: "line",
              from,
              to,
              arrow: tool.shapeKind === "arrow",
            };
            onShape(index, shape, tool.color, tool.width);
          }
        }
      }
    } else {
      onPlace(index, onPage(end));
    }
  };
  const cancel = (event: PointerEvent<HTMLDivElement>) => {
    if (event.pointerId !== pointer.current) return;
    pointer.current = null;
    follow([], false);
  };

  // The line is as thick on the screen as it will be on the page.
  const pixelsPerPoint = box.width / displayedSize(page, rotation).widthPt;
  return (
    <div
      ref={layer}
      data-page-drawing={index + 1}
      data-tool={tool.kind}
      className="absolute cursor-crosshair touch-none"
      style={{ top: box.top, left, width: box.width, height: box.height }}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={cancel}
      // What the pointer does here is the tool's: the viewer's own press and click handling (which
      // select text and choose annotations) is not.
      onMouseDown={(event) => event.stopPropagation()}
      onClick={(event) => event.stopPropagation()}
      onDoubleClick={(event) => {
        if (!isPolyTool) return;
        event.preventDefault();
        event.stopPropagation();
        commitPoly();
      }}
    >
      {isPolyTool && tool.kind === "shape" && poly.vertices.length >= 1 && (
        <svg width={box.width} height={box.height} className="pointer-events-none absolute inset-0" aria-hidden>
          {(() => {
            const strokeWidth = INK_WIDTH_PT[tool.width] * pixelsPerPoint;
            const strokeColor = INK_SWATCH[tool.color];
            const anchor = poly.vertices.at(-1)!;
            const snappedHover =
              poly.hover && poly.shift ? snapSegmentAngle(anchor, poly.hover) : poly.hover;
            const allPts = snappedHover ? [...poly.vertices, snappedHover] : poly.vertices;
            return (
              <g data-shape-preview="">
                {allPts.length >= 2 && (
                  <polyline
                    points={allPts.map((p) => `${p.x},${p.y}`).join(" ")}
                    fill="none"
                    stroke={strokeColor}
                    strokeWidth={strokeWidth}
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                )}
                {tool.shapeKind === "polygon" && allPts.length >= 3 && (
                  <line
                    x1={allPts.at(-1)!.x}
                    y1={allPts.at(-1)!.y}
                    x2={allPts[0]!.x}
                    y2={allPts[0]!.y}
                    stroke={strokeColor}
                    strokeWidth={strokeWidth}
                    strokeDasharray="4 4"
                  />
                )}
                {poly.vertices.map((p, i) => (
                  <circle
                    key={i}
                    cx={p.x}
                    cy={p.y}
                    r={Math.max(3, strokeWidth)}
                    fill={strokeColor}
                  />
                ))}
              </g>
            );
          })()}
        </svg>
      )}
      {tool.kind === "pen" && shown.points.length > 0 && (
        <svg width={box.width} height={box.height} className="pointer-events-none absolute inset-0" aria-hidden>
          <polyline
            data-stroke
            fill="none"
            stroke={INK_SWATCH[tool.color]}
            strokeWidth={INK_WIDTH_PT[tool.width] * pixelsPerPoint}
            strokeLinecap="round"
            strokeLinejoin="round"
            points={shown.points.map((point) => `${point.x},${point.y}`).join(" ")}
          />
          {shown.points.length === 1 && (
            <circle
              cx={shown.points[0]!.x}
              cy={shown.points[0]!.y}
              r={(INK_WIDTH_PT[tool.width] * pixelsPerPoint) / 2}
              fill={INK_SWATCH[tool.color]}
            />
          )}
        </svg>
      )}
      {tool.kind === "shape" && shown.points.length >= 2 && (
        <svg width={box.width} height={box.height} className="pointer-events-none absolute inset-0" aria-hidden>
          {(() => {
            const start = shown.points[0]!;
            const current = shown.points[shown.points.length - 1]!;
            const strokeWidth = INK_WIDTH_PT[tool.width] * pixelsPerPoint;
            const strokeColor = INK_SWATCH[tool.color];
            if (tool.shapeKind === "rectangle") {
              const r = shapeRect(start, current, shown.shift);
              return (
                <rect
                  data-shape-preview=""
                  x={r.x0}
                  y={r.y0}
                  width={r.x1 - r.x0}
                  height={r.y1 - r.y0}
                  fill="none"
                  stroke={strokeColor}
                  strokeWidth={strokeWidth}
                />
              );
            }
            if (tool.shapeKind === "ellipse") {
              const r = shapeRect(start, current, shown.shift);
              const rx = (r.x1 - r.x0) / 2;
              const ry = (r.y1 - r.y0) / 2;
              return (
                <ellipse
                  data-shape-preview=""
                  cx={r.x0 + rx}
                  cy={r.y0 + ry}
                  rx={rx}
                  ry={ry}
                  fill="none"
                  stroke={strokeColor}
                  strokeWidth={strokeWidth}
                />
              );
            }
            // line or arrow
            const l = shapeLine(start, current, shown.shift);
            const dx = l.to.x - l.from.x;
            const dy = l.to.y - l.from.y;
            const dist = Math.hypot(dx, dy);
            let arrowHead: ReactNode = null;
            if (tool.shapeKind === "arrow" && dist > 2) {
              const theta = Math.atan2(dy, dx);
              const armLen = Math.max(10 * pixelsPerPoint, strokeWidth * 3);
              const wing1X = l.to.x - armLen * Math.cos(theta - Math.PI / 6);
              const wing1Y = l.to.y - armLen * Math.sin(theta - Math.PI / 6);
              const wing2X = l.to.x - armLen * Math.cos(theta + Math.PI / 6);
              const wing2Y = l.to.y - armLen * Math.sin(theta + Math.PI / 6);
              arrowHead = (
                <polyline
                  points={`${wing1X},${wing1Y} ${l.to.x},${l.to.y} ${wing2X},${wing2Y}`}
                  fill="none"
                  stroke={strokeColor}
                  strokeWidth={strokeWidth}
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              );
            }
            return (
              <g data-shape-preview="">
                <line
                  x1={l.from.x}
                  y1={l.from.y}
                  x2={l.to.x}
                  y2={l.to.y}
                  stroke={strokeColor}
                  strokeWidth={strokeWidth}
                  strokeLinecap="round"
                />
                {arrowHead}
              </g>
            );
          })()}
        </svg>
      )}
      {(tool.kind === "redaction" || tool.kind === "link") && shown.points.length >= 2 && (
        <svg width={box.width} height={box.height} className="pointer-events-none absolute inset-0" aria-hidden>
          {(() => {
            const start = shown.points[0]!;
            const current = shown.points[shown.points.length - 1]!;
            const left = Math.min(start.x, current.x);
            const top = Math.min(start.y, current.y);
            const width = Math.abs(current.x - start.x);
            const height = Math.abs(current.y - start.y);
            const isLink = tool.kind === "link";
            return (
              <rect
                data-link-preview={isLink ? "" : undefined}
                data-redaction-preview={!isLink ? "" : undefined}
                x={left}
                y={top}
                width={width}
                height={height}
                fill={isLink ? "rgba(59, 130, 246, 0.2)" : "rgba(239, 68, 68, 0.2)"}
                stroke={isLink ? "rgb(59, 130, 246)" : "rgb(239, 68, 68)"}
                strokeWidth={2}
                strokeDasharray="4 2"
              />
            );
          })()}
        </svg>
      )}
    </div>
  );
}
