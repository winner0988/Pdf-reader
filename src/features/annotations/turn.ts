// Turning a picture stamp (B3-01, docs/architecture/annotations.md): the geometry of its outline
// and of the handle that is dragged to turn it. Angles are degrees, clockwise as the page shows
// them. Pure, so that it is tested without a page.

import type { Rotation } from "@/features/shell/model";
import type { Point } from "@/ipc/generated/contract";

/** What a button or a key turns a picture by, in degrees. */
export const TURN_STEP_DEGREES = 15;

/** How far the handle is above the picture's top edge, in CSS pixels. */
export const TURN_HANDLE_GAP_PX = 24;

/** How near a quarter turn the dragged picture clings to it, in degrees. */
const CLING_DEGREES = 2;

/** `degrees` as an angle above -180 and up to 180, in hundredths of a degree (and never minus zero). */
export function normalizeAngle(degrees: number): number {
  let angle = degrees % 360;
  if (angle > 180) angle -= 360;
  if (angle <= -180) angle += 360;
  return Math.round(angle * 100) / 100 + 0;
}

/**
 * The four corners of a picture of `width` by `height` turned `angle` degrees clockwise about
 * `centre`, in page space (y grows down), from the top left one when it is upright.
 */
export function turnedCorners(centre: Point, width: number, height: number, angle: number): Point[] {
  const radians = (angle * Math.PI) / 180;
  const [sin, cos] = [Math.sin(radians), Math.cos(radians)];
  return [
    [-1, -1],
    [1, -1],
    [1, 1],
    [-1, 1],
  ].map(([across, down]) => {
    const x = (across! * width) / 2;
    const y = (down! * height) / 2;
    return { x: centre.x + x * cos - y * sin, y: centre.y + x * sin + y * cos };
  });
}

/**
 * Where the handle is from the picture's centre on the screen, in CSS pixels: above the picture's
 * top edge, at `reach` from the centre along the picture's own up, which the page's `angle` and
 * the view's `rotation` turn.
 */
export function handleVector(reach: number, angle: number, rotation: Rotation): Point {
  const radians = ((angle + rotation) * Math.PI) / 180;
  return { x: reach * Math.sin(radians), y: -reach * Math.cos(radians) };
}

/** The page's angle of a picture whose handle is at (`x`, `y`) from its centre on the screen. */
export function angleOfHandle(x: number, y: number, rotation: Rotation): number {
  return normalizeAngle((Math.atan2(x, -y) * 180) / Math.PI - rotation);
}

/**
 * `angle` as it is kept while the handle is dragged: a multiple of `TURN_STEP_DEGREES` when
 * `stepped` (Shift), else free, except that a quarter turn is not missed by a hair.
 */
export function snapAngle(angle: number, stepped: boolean): number {
  if (stepped) return normalizeAngle(Math.round(angle / TURN_STEP_DEGREES) * TURN_STEP_DEGREES);
  const quarter = Math.round(angle / 90) * 90;
  return normalizeAngle(Math.abs(angle - quarter) <= CLING_DEGREES ? quarter : angle);
}
