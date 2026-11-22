import { describe, expect, it } from "vitest";

import { calculateTypedFontSize, findCropBounds } from "./signatureImage";

describe("signatureImage helpers", () => {
  describe("findCropBounds", () => {
    it("returns null for completely transparent buffer", () => {
      const width = 100;
      const height = 50;
      const pixels = new Uint8ClampedArray(width * height * 4);
      expect(findCropBounds(pixels, width, height)).toBeNull();
    });

    it("crops tight bounding box with padding clamped to buffer dimensions", () => {
      const width = 100;
      const height = 100;
      const pixels = new Uint8ClampedArray(width * height * 4);

      // Draw a 10x10 square from (30, 20) to (39, 29)
      for (let y = 20; y <= 29; y++) {
        for (let x = 30; x <= 39; x++) {
          pixels[(y * width + x) * 4 + 3] = 255; // alpha
        }
      }

      // With default padding = 12:
      // x: max(0, 30 - 12) = 18
      // y: max(0, 20 - 12) = 8
      // x1: min(100, 39 + 1 + 12) = 52
      // y1: min(100, 29 + 1 + 12) = 42
      // width = 52 - 18 = 34, height = 42 - 8 = 34
      const bounds = findCropBounds(pixels, width, height, 12);
      expect(bounds).toEqual({
        x: 18,
        y: 8,
        width: 34,
        height: 34,
      });
    });

    it("clamps padding when ink is near borders", () => {
      const width = 50;
      const height = 50;
      const pixels = new Uint8ClampedArray(width * height * 4);

      // Ink at pixel (2, 2)
      pixels[(2 * width + 2) * 4 + 3] = 255;

      const bounds = findCropBounds(pixels, width, height, 12);
      expect(bounds).toEqual({
        x: 0,
        y: 0,
        width: 15, // 2 + 1 + 12 = 15
        height: 15,
      });
    });
  });

  describe("calculateTypedFontSize", () => {
    it("returns baseFontSize when measured width is within maxWidth", () => {
      expect(calculateTypedFontSize(500, 72, 1024)).toBe(72);
      expect(calculateTypedFontSize(1024, 72, 1024)).toBe(72);
    });

    it("scales down proportionally when measured width exceeds maxWidth", () => {
      // 2048px measured -> scaled down by half: 72 * 1024 / 2048 = 36
      expect(calculateTypedFontSize(2048, 72, 1024)).toBe(36);
    });

    it("does not go below minimum font size (12)", () => {
      expect(calculateTypedFontSize(100000, 72, 1024)).toBe(12);
    });
  });
});
