import { describe, expect, it } from "vitest";

import {
  angleOfHandle,
  handleVector,
  normalizeAngle,
  snapAngle,
  turnedCorners,
  TURN_STEP_DEGREES,
} from "@/features/annotations/turn";

describe("angles of a turned picture (B3-01)", () => {
  it("are kept above -180 and up to 180, to hundredths, and never minus zero", () => {
    expect(normalizeAngle(0)).toBe(0);
    expect(Object.is(normalizeAngle(-0), 0)).toBe(true);
    expect(normalizeAngle(180)).toBe(180);
    expect(normalizeAngle(-180)).toBe(180);
    expect(normalizeAngle(190)).toBe(-170);
    expect(normalizeAngle(-190)).toBe(170);
    expect(normalizeAngle(540)).toBe(180);
    expect(normalizeAngle(360)).toBe(0);
    expect(normalizeAngle(14.9999)).toBe(15);
    expect(normalizeAngle(-0.004)).toBe(0);
  });

  it("turn the corners clockwise about the centre, as the page shows it (y grows down)", () => {
    const upright = turnedCorners({ x: 50, y: 20 }, 100, 40, 0);
    expect(upright).toEqual([
      { x: 0, y: 0 },
      { x: 100, y: 0 },
      { x: 100, y: 40 },
      { x: 0, y: 40 },
    ]);
    // A quarter turn clockwise: what was at the left is at the top, the top is at the right.
    const quarter = turnedCorners({ x: 50, y: 20 }, 100, 40, 90);
    const expected = [
      { x: 70, y: -30 },
      { x: 70, y: 70 },
      { x: 30, y: 70 },
      { x: 30, y: -30 },
    ];
    quarter.forEach((corner, at) => {
      expect(corner.x).toBeCloseTo(expected[at]!.x);
      expect(corner.y).toBeCloseTo(expected[at]!.y);
    });
  });

  it("put the handle above the picture's top edge, wherever the picture and the view are turned", () => {
    const at = (reach: number, angle: number, rotation: 0 | 90 | 180 | 270) => handleVector(reach, angle, rotation);
    expect(at(50, 0, 0).x).toBeCloseTo(0);
    expect(at(50, 0, 0).y).toBeCloseTo(-50);
    // Turned a quarter clockwise, its top is on the right.
    expect(at(50, 90, 0).x).toBeCloseTo(50);
    expect(at(50, 90, 0).y).toBeCloseTo(0);
    // The view turned a quarter shows an upright picture's top on the right too.
    expect(at(50, 0, 90).x).toBeCloseTo(50);
    expect(at(50, 0, 180).y).toBeCloseTo(50);
  });

  it("are read back from where the handle is", () => {
    const rotation = 0;
    expect(angleOfHandle(0, -50, rotation)).toBe(0);
    expect(angleOfHandle(50, 0, rotation)).toBe(90);
    expect(angleOfHandle(0, 50, rotation)).toBe(180);
    expect(angleOfHandle(-50, 0, rotation)).toBe(-90);
    // The handle is where the view puts it: the view's quarter turn is not the picture's.
    expect(angleOfHandle(50, 0, 90)).toBe(0);
    // What handleVector says, angleOfHandle reads.
    for (const angle of [-170, -45, 0, 33, 90, 179]) {
      for (const view of [0, 90, 180, 270] as const) {
        const handle = handleVector(80, angle, view);
        expect(angleOfHandle(handle.x, handle.y, view)).toBeCloseTo(angle, 1);
      }
    }
  });

  it("are free while the handle is dragged, cling to a quarter turn, and go by a step with Shift", () => {
    expect(snapAngle(37.3, false)).toBe(37.3);
    expect(snapAngle(88.5, false)).toBe(90);
    expect(snapAngle(87, false)).toBe(87);
    expect(snapAngle(-179, false)).toBe(180);
    expect(snapAngle(22, true)).toBe(TURN_STEP_DEGREES);
    expect(snapAngle(23, true)).toBe(30);
    expect(snapAngle(-7, true)).toBe(0);
    expect(snapAngle(178, true)).toBe(180);
  });
});
