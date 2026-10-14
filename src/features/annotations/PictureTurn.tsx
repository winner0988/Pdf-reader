// The outline of a chosen picture stamp as it is turned, and the handle above it that is dragged
// to turn it (B3-01, docs/architecture/annotations.md). While the pointer is down the outline
// follows it; the edit is sent when the pointer is released. Like the handles that resize a
// stamp (MoveResize) the handle is for the pointer: the buttons in the stamp's toolbar and the
// keys [ and ] turn it without one.

import { useState, type PointerEvent } from "react";

import {
  angleOfHandle,
  handleVector,
  normalizeAngle,
  snapAngle,
  turnedCorners,
  TURN_HANDLE_GAP_PX,
} from "@/features/annotations/turn";
import type { PageSize, Rotation } from "@/features/shell/model";
import { displayedSize, pageToBox, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import type { PictureView, Rect } from "@/ipc/generated/contract";

type PictureTurnProps = {
  picture: PictureView;
  /** The box that holds the turned picture (page space), as listed. */
  rect: Rect;
  page: PageSize;
  rotation: Rotation;
  box: PageBox;
  /** The picture is to be turned to `degrees` clockwise from upright. */
  onTurn: (degrees: number) => void;
};

/** The handle's size, in CSS pixels. */
const HANDLE_PX = 12;

type Drag = {
  pointer: number;
  /** Where the pointer was pressed, and where the handle was from the centre then. */
  x: number;
  y: number;
  from: { x: number; y: number };
  angle: number;
};

export function PictureTurn({ picture, rect, page, rotation, box, onTurn }: PictureTurnProps) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const pixelsPerPoint = box.width / displayedSize(page, rotation).widthPt;
  const centre = { x: (rect.x0 + rect.x1) / 2, y: (rect.y0 + rect.y1) / 2 };
  const angle = drag?.angle ?? picture.angle;
  const middle = pageToBox(centre, page, rotation, box);
  const corners = turnedCorners(centre, picture.width, picture.height, angle).map((corner) =>
    pageToBox(corner, page, rotation, box),
  );
  // The picture's own height, as the screen shows it, whichever way it and the view are turned.
  const half = (picture.height * pixelsPerPoint) / 2;
  const edge = handleVector(half, angle, rotation);
  const pull = handleVector(half + TURN_HANDLE_GAP_PX, angle, rotation);
  const handle = { x: middle.x + pull.x, y: middle.y + pull.y };

  const start = (event: PointerEvent<HTMLElement>) => {
    if (event.button !== 0 || drag) return;
    // No text is selected by this press, and nothing chosen by its click.
    event.preventDefault();
    event.stopPropagation();
    // Captured by the handle: its moves and its release come to it, wherever the pointer goes.
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDrag({ pointer: event.pointerId, x: event.clientX, y: event.clientY, from: pull, angle: picture.angle });
  };
  const move = (event: PointerEvent<HTMLElement>) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    const x = drag.from.x + event.clientX - drag.x;
    const y = drag.from.y + event.clientY - drag.y;
    // The pointer on the picture's centre says nothing of an angle: it keeps the last.
    if (Math.hypot(x, y) < 4) return;
    setDrag({ ...drag, angle: snapAngle(angleOfHandle(x, y, rotation), event.shiftKey) });
  };
  const end = (event: PointerEvent<HTMLElement>) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    setDrag(null);
    if (Math.abs(normalizeAngle(drag.angle - picture.angle)) > 0.05) onTurn(drag.angle);
  };
  const cancel = (event: PointerEvent<HTMLElement>) => {
    if (drag && event.pointerId === drag.pointer) setDrag(null);
  };

  return (
    <>
      <svg
        aria-hidden
        className="pointer-events-none absolute top-0 left-0 overflow-visible"
        width={box.width}
        height={box.height}
      >
        <polygon
          data-turn-outline
          points={corners.map((corner) => `${corner.x},${corner.y}`).join(" ")}
          className="fill-none stroke-primary"
          strokeWidth={1.5}
        />
        <line
          x1={middle.x + edge.x}
          y1={middle.y + edge.y}
          x2={handle.x}
          y2={handle.y}
          className="stroke-primary"
          strokeWidth={1.5}
        />
      </svg>
      <div
        data-turn-handle
        title={strings.annotations.turnHandle}
        aria-hidden
        className="pointer-events-auto absolute cursor-grab touch-none rounded-full border border-primary bg-background"
        style={{ left: handle.x - HANDLE_PX / 2, top: handle.y - HANDLE_PX / 2, width: HANDLE_PX, height: HANDLE_PX }}
        onPointerDown={start}
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={cancel}
      />
    </>
  );
}
