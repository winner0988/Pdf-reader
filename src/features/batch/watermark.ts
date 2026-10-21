// What is asked for when a watermark is drawn on copies in a batch (B3-04, docs/architecture/watermark.md):
// rules match ipc_contract validation, so the dialog can say what is wrong before anything is sent.

import { strings } from "@/i18n/zh-TW";
import type {
  BatchOperation,
  Rgb,
  Watermark,
  WatermarkPages,
  WatermarkPlace,
} from "@/ipc/generated/contract";

export type WatermarkForm = {
  type: "text" | "picture";
  text: string;
  size: number;
  color: string;
  pictureWidth: number;
  opacity: number;
  diagonal: boolean;
  place: WatermarkPlace;
  pages: WatermarkPages;
};

export const DEFAULT_WATERMARK_FORM: WatermarkForm = {
  type: "text",
  text: "機密",
  size: 40,
  color: "#c80000",
  pictureWidth: 150,
  opacity: 50,
  diagonal: true,
  place: "center",
  pages: "all",
};

export type WatermarkProblem =
  | "emptyText"
  | "textTooLong"
  | "sizeOutOfRange"
  | "widthOutOfRange"
  | "opacityOutOfRange";

/** The first thing wrong with `form`, or `null` if it can be sent. */
export function watermarkProblemWith(form: WatermarkForm): WatermarkProblem | null {
  if (Number.isNaN(form.opacity) || form.opacity < 5 || form.opacity > 100) {
    return "opacityOutOfRange";
  }
  if (form.type === "text") {
    if (form.text.trim() === "") return "emptyText";
    if (form.text.length > 100) return "textTooLong";
    if (Number.isNaN(form.size) || form.size < 8 || form.size > 200) {
      return "sizeOutOfRange";
    }
  } else {
    if (
      Number.isNaN(form.pictureWidth) ||
      form.pictureWidth < 16 ||
      form.pictureWidth > 1200
    ) {
      return "widthOutOfRange";
    }
  }
  return null;
}

/** What the user is told about `problem`. */
export function describeWatermarkProblem(problem: WatermarkProblem): string {
  return strings.batch.watermarkProblem[problem];
}

export function hexToRgb(hex: string): Rgb {
  const clean = hex.replace("#", "");
  const num = parseInt(clean, 16);
  if (Number.isNaN(num) || clean.length !== 6) {
    return { r: 200, g: 0, b: 0 };
  }
  return {
    r: (num >> 16) & 255,
    g: (num >> 8) & 255,
    b: num & 255,
  };
}

export function rgbToHex(rgb: Rgb): string {
  const pad = (n: number) => n.toString(16).padStart(2, "0");
  return `#${pad(rgb.r)}${pad(rgb.g)}${pad(rgb.b)}`;
}

/** The watermark operation of a batch (B3-04) for a form with nothing wrong with it. */
export function batchWatermarkOf(form: WatermarkForm): BatchOperation {
  const watermark: Watermark = {
    content:
      form.type === "text"
        ? {
            kind: "text",
            text: form.text,
            size: form.size,
            color: hexToRgb(form.color),
          }
        : {
            kind: "picture",
            widthPt: form.pictureWidth,
          },
    opacity: Math.round(form.opacity),
    diagonal: form.diagonal,
    place: form.place,
    pages: form.pages,
  };
  return {
    kind: "watermark",
    watermark,
  };
}
