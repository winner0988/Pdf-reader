import { describe, expect, it } from "vitest";

import {
  colorName,
  families,
  fontNamed,
  fromHex,
  sizeAllowed,
  startingFamily,
  TEXT_COLORS,
  textTooLong,
  toHex,
  variantOf,
} from "@/features/annotations/textBox";
import type { FontInfo } from "@/ipc/generated/contract";

const font = (id: number, name: string, family: string, bold = false, italic = false): FontInfo => ({
  id,
  name,
  family,
  bold,
  italic,
  source: "installed",
});
const FONTS = [
  font(1, "Arial", "Arial"),
  font(2, "Arial Bold", "Arial", true),
  font(3, "Segoe UI", "Segoe UI"),
  font(4, "Microsoft JhengHei UI", "Microsoft JhengHei UI"),
];

describe("fonts", () => {
  it("are grouped into families, in the order the list has them", () => {
    expect(families(FONTS)).toEqual(["Arial", "Segoe UI", "Microsoft JhengHei UI"]);
    expect(families([])).toEqual([]);
  });

  it("have a bold or an italic face only where the list says so", () => {
    expect(variantOf(FONTS, "Arial", true, false)?.name).toBe("Arial Bold");
    expect(variantOf(FONTS, "Arial", false, false)?.name).toBe("Arial");
    // No italic and no bold italic: nothing is made up.
    expect(variantOf(FONTS, "Arial", false, true)).toBeUndefined();
    expect(variantOf(FONTS, "Segoe UI", true, false)).toBeUndefined();
    expect(variantOf(FONTS, "Nowhere", false, false)).toBeUndefined();
    expect(fontNamed(FONTS, "Arial Bold")?.id).toBe(2);
    expect(fontNamed(FONTS, "Arial Black")).toBeUndefined();
  });

  it("start a new box with the family used last, else one that covers Chinese, else the first", () => {
    expect(startingFamily(FONTS, "Segoe UI")).toBe("Segoe UI");
    // The last one is gone: the JhengHei family (UI or not) comes first of the preferred ones.
    expect(startingFamily(FONTS, "Gone")).toBe("Microsoft JhengHei UI");
    expect(startingFamily(FONTS)).toBe("Microsoft JhengHei UI");
    expect(startingFamily(FONTS.slice(0, 2))).toBe("Arial");
    expect(startingFamily([font(9, "Zapfino", "Zapfino")])).toBe("Zapfino");
    expect(startingFamily([])).toBeUndefined();
  });
});

describe("colours", () => {
  it("go to and from the hex text of a colour picker", () => {
    expect(toHex({ r: 26, g: 77, b: 217 })).toBe("#1a4dd9");
    expect(toHex({ r: 0, g: 0, b: 0 })).toBe("#000000");
    expect(fromHex("#1A4dd9")).toEqual({ r: 26, g: 77, b: 217 });
    for (const bad of ["", "#fff", "1a4dd9", "#1a4dd", "#1a4dd9ff", "#gg0000"]) expect(fromHex(bad)).toBeNull();
    for (const preset of TEXT_COLORS) expect(fromHex(toHex(preset.rgb))).toEqual(preset.rgb);
  });

  it("are named when they are one of the presets", () => {
    expect(colorName({ r: 0, g: 0, b: 0 })).toBe("black");
    expect(colorName(TEXT_COLORS[2].rgb)).toBe("blue");
    expect(colorName({ r: 1, g: 2, b: 3 })).toBeNull();
  });
});

describe("what a box can say", () => {
  it("is within the size the main process takes, in bytes of UTF-8", () => {
    expect(textTooLong("a".repeat(4096))).toBe(false);
    expect(textTooLong("a".repeat(4097))).toBe(true);
    // A Chinese character is three bytes.
    expect(textTooLong("字".repeat(1365))).toBe(false);
    expect(textTooLong("字".repeat(1366))).toBe(true);
  });

  it("is set in a size from 4 to 288 points", () => {
    for (const ok of [4, 12, 14.5, 288]) expect(sizeAllowed(ok)).toBe(true);
    for (const bad of [0, 3.9, 288.1, Number.NaN, Number.POSITIVE_INFINITY, -12]) expect(sizeAllowed(bad)).toBe(false);
  });
});
