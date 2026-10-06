// The layer over a page that takes the pointer while a tool is chosen (B2-08,
// docs/architecture/annotations.md): the pen draws a stroke, which is shown as it is drawn and
// sent when the pointer is released; a stamp is put down where the pointer is released. The layer
// is above the page's other layers, so that nothing under it (text, links, fields) gets the press.

import { useRef, useState, type PointerEvent } from "react";

import { INK_SWATCH, INK_WIDTH_PT, keptPointOnPage, strokePoints, type Tool } from "@/features/annotations/tools";
import type { PageSize, Rotation } from "@/features/shell/model";
import { boxToPage, displayedSize, type PageBox } from "@/features/viewer/layout";
import type { Point } from "@/ipc/generated/contract";

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
};

export function PageDrawing({ tool, index, page, rotation, box, left, onStroke, onPlace }: PageDrawingProps) {
  const layer = useRef<HTMLDivElement>(null);
  /** The pointer that is drawing, and where it has been, in CSS pixels of the page's box. */
  const pointer = useRef<number | null>(null);
  const trail = useRef<Point[]>([]);
  /** What is shown of it: the trail, replaced (not changed) with every move. */
  const [shown, setShown] = useState<Point[]>([]);
  const follow = (points: Point[]) => {
    trail.current = points;
    setShown(points);
  };

  const inBox = (event: PointerEvent): Point => {
    const bounds = layer.current?.getBoundingClientRect();
    return { x: event.clientX - (bounds?.left ?? 0), y: event.clientY - (bounds?.top ?? 0) };
  };
  // The pointer can leave the page while it is down: what is drawn or put down stays on the page.
  const onPage = (point: Point) => keptPointOnPage(boxToPage(point, page, rotation, box), page);

  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || pointer.current !== null) return;
    // No text is selected by this press, and no annotation chosen by its click.
    event.preventDefault();
    pointer.current = event.pointerId;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    follow([inBox(event)]);
  };
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (event.pointerId !== pointer.current) return;
    follow([...trail.current, inBox(event)]);
  };
  const up = (event: PointerEvent<HTMLDivElement>) => {
    if (event.pointerId !== pointer.current) return;
    pointer.current = null;
    const end = inBox(event);
    const points = [...trail.current, end];
    follow([]);
    if (tool.kind === "pen") onStroke(index, strokePoints(points.map(onPage)));
    else onPlace(index, onPage(end));
  };
  const cancel = (event: PointerEvent<HTMLDivElement>) => {
    if (event.pointerId !== pointer.current) return;
    pointer.current = null;
    follow([]);
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
    >
      {tool.kind === "pen" && shown.length > 0 && (
        <svg width={box.width} height={box.height} className="pointer-events-none absolute inset-0" aria-hidden>
          <polyline
            data-stroke
            fill="none"
            stroke={INK_SWATCH[tool.color]}
            strokeWidth={INK_WIDTH_PT[tool.width] * pixelsPerPoint}
            strokeLinecap="round"
            strokeLinejoin="round"
            points={shown.map((point) => `${point.x},${point.y}`).join(" ")}
          />
          {shown.length === 1 && (
            <circle
              cx={shown[0]!.x}
              cy={shown[0]!.y}
              r={(INK_WIDTH_PT[tool.width] * pixelsPerPoint) / 2}
              fill={INK_SWATCH[tool.color]}
            />
          )}
        </svg>
      )}
    </div>
  );
}
