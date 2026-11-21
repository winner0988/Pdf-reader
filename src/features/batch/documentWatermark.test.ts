import { describe, expect, it } from "vitest";

import {
  editOf,
  pagesOf,
  placement,
} from "./documentWatermark";
import type { WatermarkForm } from "./watermark";
import type { PageSize, StampImageId } from "@/ipc/generated/contract";

describe("documentWatermark", () => {
  describe("pagesOf", () => {
    it("returns all pages for 'all'", () => {
      expect(pagesOf("all", "", 5)).toEqual([0, 1, 2, 3, 4]);
    });

    it("returns first page for 'first'", () => {
      expect(pagesOf("first", "", 5)).toEqual([0]);
    });

    it("returns all but first for 'allButFirst'", () => {
      expect(pagesOf("allButFirst", "", 5)).toEqual([1, 2, 3, 4]);
      expect(pagesOf("allButFirst", "", 1)).toEqual([]);
    });

    it("parses valid ranges for 'range'", () => {
      expect(pagesOf("range", "1-3, 5", 5)).toEqual([0, 1, 2, 4]);
      expect(pagesOf("range", "2", 5)).toEqual([1]);
    });

    it("returns null for invalid or out of bounds range", () => {
      expect(pagesOf("range", "1-10", 5)).toBeNull();
      expect(pagesOf("range", "abc", 5)).toBeNull();
      expect(pagesOf("range", "", 5)).toBeNull();
    });

    it("returns null for count <= 0", () => {
      expect(pagesOf("all", "", 0)).toBeNull();
      expect(pagesOf("first", "", -1)).toBeNull();
    });
  });

  describe("placement", () => {
    const page: PageSize = { widthPt: 600, heightPt: 800 };

    it("computes center horizontal placement", () => {
      const p = placement(page, { width: 200, height: 50 }, false, "center");
      expect(p.center.x).toBe(300);
      expect(p.center.y).toBe(400);
      expect(p.angle).toBe(0);
      expect(p.scale).toBe(1.0);
    });

    it("computes top and bottom horizontal placement with margin and reach", () => {
      const top = placement(page, { width: 200, height: 50 }, false, "top");
      expect(top.center.x).toBe(300);
      // reach = 50 / 2 = 25; margin = 36; y = 36 + 25 = 61
      expect(top.center.y).toBeCloseTo(61, 1);

      const bottom = placement(page, { width: 200, height: 50 }, false, "bottom");
      expect(bottom.center.x).toBe(300);
      // y = 800 - 36 - 25 = 739
      expect(bottom.center.y).toBeCloseTo(739, 1);
    });

    it("scales down when mark width exceeds room (FIT = 0.9)", () => {
      // room = 600 * 0.9 = 540; mark width = 1080 -> scale = 0.5
      const p = placement(page, { width: 1080, height: 100 }, false, "center");
      expect(p.scale).toBeCloseTo(0.5, 2);
    });

    it("computes diagonal angle using atan2", () => {
      const p = placement(page, { width: 200, height: 50 }, true, "center");
      expect(p.angle).toBeCloseTo(Math.atan2(800, 600), 4);
    });
  });

  describe("editOf", () => {
    const textForm: WatermarkForm = {
      type: "text",
      text: "DRAFT",
      size: 48,
      color: "#ff0000",
      pictureWidth: 150,
      opacity: 60,
      diagonal: true,
      place: "center",
      pages: "first", // note: editOf forces pages: "all" on the watermark struct
    };

    it("creates AddWatermark edit for text watermark", () => {
      const edit = editOf(textForm, [0, 1], null);
      expect(edit).toEqual({
        kind: "addWatermark",
        pages: [0, 1],
        watermark: {
          content: {
            kind: "text",
            text: "DRAFT",
            size: 48,
            color: { r: 255, g: 0, b: 0 },
          },
          opacity: 60,
          diagonal: true,
          place: "center",
          pages: "all",
        },
        image: null,
      });
    });

    it("creates AddWatermark edit for picture watermark", () => {
      const picForm: WatermarkForm = {
        ...textForm,
        type: "picture",
        pictureWidth: 200,
      };
      const edit = editOf(picForm, [0], 42 as StampImageId);
      expect(edit).toEqual({
        kind: "addWatermark",
        pages: [0],
        watermark: {
          content: {
            kind: "picture",
            widthPt: 200,
          },
          opacity: 60,
          diagonal: true,
          place: "center",
          pages: "all",
        },
        image: 42,
      });
    });
  });
});
