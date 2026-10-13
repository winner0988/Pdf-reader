// What the page knows of a text box (B2-30, docs/architecture/text-box-engine.md): the fonts it can
// choose from, grouped into families (bold and italic are other fonts of a family, never made up),
// the colours it offers, and what makes the size acceptable (the words are cleaned as a note's are,
// `noteText`). Pure, so that it is tested without a dialog.

import {
  LIMITS,
  type FontInfo,
  type Rgb,
  type TextAlign,
} from "@/ipc/generated/contract";

/** The size a new text box sets its words in, in points. */
export const DEFAULT_TEXT_SIZE = 14;

/** The colours the dialog offers by name; any other is chosen with the colour picker. */
export const TEXT_COLORS = [
  { id: "black", rgb: { r: 0, g: 0, b: 0 } },
  { id: "red", rgb: { r: 200, g: 30, b: 30 } },
  { id: "blue", rgb: { r: 26, g: 77, b: 217 } },
  { id: "green", rgb: { r: 26, g: 130, b: 51 } },
  { id: "orange", rgb: { r: 230, g: 126, b: 0 } },
  { id: "purple", rgb: { r: 120, g: 50, b: 180 } },
  { id: "gray", rgb: { r: 110, g: 110, b: 110 } },
] as const satisfies readonly { id: string; rgb: Rgb }[];

export type TextColorId = (typeof TEXT_COLORS)[number]["id"];

export const TEXT_ALIGNS: TextAlign[] = ["left", "center", "right"];

/** The name of a preset colour, if `rgb` is one. */
export function colorName(rgb: Rgb): TextColorId | null {
  const found = TEXT_COLORS.find(({ rgb: preset }) => preset.r === rgb.r && preset.g === rgb.g && preset.b === rgb.b);
  return found?.id ?? null;
}

const channel = (value: number) => value.toString(16).padStart(2, "0");

/** `rgb` as the `#rrggbb` an `<input type="color">` takes. */
export function toHex(rgb: Rgb): string {
  return `#${channel(rgb.r)}${channel(rgb.g)}${channel(rgb.b)}`;
}

/** The colour of `#rrggbb`; `null` for anything else. */
export function fromHex(hex: string): Rgb | null {
  const match = /^#([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})$/.exec(hex);
  if (!match) return null;
  return { r: parseInt(match[1]!, 16), g: parseInt(match[2]!, 16), b: parseInt(match[3]!, 16) };
}

/** The families of `fonts`, in the order the list first has them. */
export function families(fonts: readonly FontInfo[]): string[] {
  const seen = new Set<string>();
  for (const font of fonts) seen.add(font.family);
  return [...seen];
}

/**
 * The font of `family` that is regular, bold, italic or both, as asked: the one the list has, or
 * `undefined`. A family that has no italic has none: the page does not slant a regular face, and
 * the worker would not draw it.
 */
export function variantOf(
  fonts: readonly FontInfo[],
  family: string,
  bold: boolean,
  italic: boolean,
): FontInfo | undefined {
  return fonts.find((font) => font.family === family && font.bold === bold && font.italic === italic);
}

/** Families that cover Chinese and Japanese, and read well on paper, to start from. */
const PREFERRED = ["Microsoft JhengHei", "Microsoft YaHei", "Yu Gothic", "Segoe UI", "Arial"];

/**
 * The family a new text box starts with: the one used last if it is still installed, else the first
 * of a few that cover Chinese and Japanese, else the first of the list.
 */
export function startingFamily(fonts: readonly FontInfo[], last?: string): string | undefined {
  const all = families(fonts);
  if (last && all.includes(last)) return last;
  const lower = (name: string) => name.toLowerCase();
  for (const wanted of PREFERRED) {
    const found = all.find((family) => lower(family).startsWith(lower(wanted)));
    if (found) return found;
  }
  return all[0];
}

/** The font of the list whose name is `name` (what Windows calls it), if there is one. */
export function fontNamed(fonts: readonly FontInfo[], name: string): FontInfo | undefined {
  return fonts.find((font) => font.name === name);
}

/** Whether `text` is more than a text box takes (UTF-8 bytes, as for a note). */
export function textTooLong(text: string): boolean {
  return new TextEncoder().encode(text).length > LIMITS.maxTextBoxBytes;
}

/** Whether `size` is one a text box can be set in. */
export function sizeAllowed(size: number): boolean {
  return Number.isFinite(size) && size >= LIMITS.minTextBoxSize && size <= LIMITS.maxTextBoxSize;
}
