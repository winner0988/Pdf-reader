import { describe, expect, it } from "vitest";

import { rotate, stepZoom } from "@/features/shell/model";

describe("stepZoom", () => {
  it("moves to the next level in either direction", () => {
    expect(stepZoom(100, 1)).toBe(110);
    expect(stepZoom(100, -1)).toBe(90);
    expect(stepZoom(125, 1)).toBe(150);
    // Fit modes continue from what they show, not from 100%.
    expect(stepZoom("fitWidth", 1, 137)).toBe(150);
    expect(stepZoom("fitPage", -1, 137)).toBe(125);
  });

  it("steps from 100% when a fit mode is active", () => {
    expect(stepZoom("fitWidth", 1)).toBe(110);
    expect(stepZoom("fitPage", -1)).toBe(90);
  });

  it("snaps an in-between value to the neighbouring level", () => {
    expect(stepZoom(105, 1)).toBe(110);
    expect(stepZoom(105, -1)).toBe(100);
  });

  it("stays within 25% to 800%", () => {
    expect(stepZoom(800, 1)).toBe(800);
    expect(stepZoom(25, -1)).toBe(25);
  });
});

describe("rotate", () => {
  it("turns in 90 degree steps and wraps around", () => {
    expect(rotate(0, 1)).toBe(90);
    expect(rotate(270, 1)).toBe(0);
    expect(rotate(0, -1)).toBe(270);
  });
});
