import { describe, expect, it } from "vitest";

import {
  batchWatermarkOf,
  DEFAULT_WATERMARK_FORM,
  describeWatermarkProblem,
  hexToRgb,
  rgbToHex,
  watermarkProblemWith,
  type WatermarkForm,
} from "@/features/batch/watermark";
import { strings } from "@/i18n/zh-TW";

describe("batch watermark form", () => {
  it("converts hex to rgb and back", () => {
    expect(hexToRgb("#c80000")).toEqual({ r: 200, g: 0, b: 0 });
    expect(hexToRgb("#00ff00")).toEqual({ r: 0, g: 255, b: 0 });
    expect(hexToRgb("invalid")).toEqual({ r: 200, g: 0, b: 0 });
    expect(rgbToHex({ r: 200, g: 0, b: 0 })).toBe("#c80000");
  });

  it("validates form values", () => {
    expect(watermarkProblemWith(DEFAULT_WATERMARK_FORM)).toBeNull();

    // empty text
    expect(
      watermarkProblemWith({ ...DEFAULT_WATERMARK_FORM, text: "   " }),
    ).toBe("emptyText");

    // text too long
    expect(
      watermarkProblemWith({
        ...DEFAULT_WATERMARK_FORM,
        text: "x".repeat(101),
      }),
    ).toBe("textTooLong");

    // size out of range
    expect(
      watermarkProblemWith({ ...DEFAULT_WATERMARK_FORM, size: 7 }),
    ).toBe("sizeOutOfRange");
    expect(
      watermarkProblemWith({ ...DEFAULT_WATERMARK_FORM, size: 201 }),
    ).toBe("sizeOutOfRange");

    // opacity out of range
    expect(
      watermarkProblemWith({ ...DEFAULT_WATERMARK_FORM, opacity: 4 }),
    ).toBe("opacityOutOfRange");
    expect(
      watermarkProblemWith({ ...DEFAULT_WATERMARK_FORM, opacity: 101 }),
    ).toBe("opacityOutOfRange");

    // picture width out of range
    const pictureForm: WatermarkForm = {
      ...DEFAULT_WATERMARK_FORM,
      type: "picture",
      pictureWidth: 150,
    };
    expect(watermarkProblemWith(pictureForm)).toBeNull();
    expect(
      watermarkProblemWith({ ...pictureForm, pictureWidth: 15 }),
    ).toBe("widthOutOfRange");
    expect(
      watermarkProblemWith({ ...pictureForm, pictureWidth: 1201 }),
    ).toBe("widthOutOfRange");
  });

  it("describes problems in traditional Chinese", () => {
    expect(describeWatermarkProblem("emptyText")).toBe(
      strings.batch.watermarkProblem.emptyText,
    );
    expect(describeWatermarkProblem("textTooLong")).toBe(
      strings.batch.watermarkProblem.textTooLong,
    );
  });

  it("converts form to BatchOperation", () => {
    const textOp = batchWatermarkOf(DEFAULT_WATERMARK_FORM);
    expect(textOp).toEqual({
      kind: "watermark",
      watermark: {
        content: {
          kind: "text",
          text: "機密",
          size: 40,
          color: { r: 200, g: 0, b: 0 },
        },
        opacity: 50,
        diagonal: true,
        place: "center",
        pages: "all",
      },
    });

    const picOp = batchWatermarkOf({
      ...DEFAULT_WATERMARK_FORM,
      type: "picture",
      pictureWidth: 200,
      diagonal: false,
      place: "top",
      pages: "first",
    });
    expect(picOp).toEqual({
      kind: "watermark",
      watermark: {
        content: {
          kind: "picture",
          widthPt: 200,
        },
        opacity: 50,
        diagonal: false,
        place: "top",
        pages: "first",
      },
    });
  });
});
