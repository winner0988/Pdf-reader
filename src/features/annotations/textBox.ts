// What the page knows of a text box (B2-30, #719, docs/architecture/text-boxes.md): the fonts it
// can choose from, grouped into families (imported fonts first; bold and italic are other fonts of
// a family, never made up), the colours it offers, layout geometry for inline typing, and whether
// WebView2 can render a chosen font directly while typing. Pure, so that it is tested without a page.

import { keptOnPage } from "@/features/annotations/tools";
import type { PageSize } from "@/features/shell/model";
import {
  LIMITS,
  type FontInfo,
  type Point,
  type Rect,
  type Rgb,
  type TextAlign,
  type TextBoxStyle,
} from "@/ipc/generated/contract";

/** The size a new text box sets its words in, in points. */
export const DEFAULT_TEXT_SIZE = 14;

/** Default line step (in thousandths of an em) before `prepare_text_box` returns the font's own. */
export const DEFAULT_LINE_STEP = 1200;

/** Padding inside a text box on left, right, and top, in points (matches `pdf_worker` `PADDING`). */
export const TEXT_BOX_PADDING_PT = 2;

/** Margin kept from the right and bottom edges of the page when auto-growing a new text box. */
export const TEXT_BOX_EDGE_MARGIN_PT = 8;

/** Initial minimum width of a newly placed inline text box, in points, before typing widens it. */
export const INITIAL_INLINE_WIDTH_PT = 120;

/** The colours the toolbar offers by name; any other is chosen with the colour picker. */
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

/** The families of `fonts`, with imported families first and installed families in list order. */
export function families(fonts: readonly FontInfo[]): string[] {
  const imported = new Set<string>();
  const installed = new Set<string>();
  for (const font of fonts) {
    if (font.source === "imported") imported.add(font.family);
    else installed.add(font.family);
  }
  const result = [...imported];
  for (const family of installed) {
    if (!imported.has(family)) result.push(family);
  }
  return result;
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

/** The font of `family` as bold and italic are asked, or the nearest it has. */
export function settleVariant(
  fonts: readonly FontInfo[],
  family: string,
  bold: boolean,
  italic: boolean,
): { font: FontInfo; bold: boolean; italic: boolean } | undefined {
  const exact = variantOf(fonts, family, bold, italic);
  if (exact) return { font: exact, bold, italic };
  const regular = variantOf(fonts, family, false, false) ?? fonts.find((font) => font.family === family);
  return regular ? { font: regular, bold: regular.bold, italic: regular.italic } : undefined;
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

/** Whether two styles are identical. */
export function sameStyle(one: TextBoxStyle, other: TextBoxStyle): boolean {
  return (
    one.size === other.size &&
    one.align === other.align &&
    one.color.r === other.color.r &&
    one.color.g === other.color.g &&
    one.color.b === other.color.b
  );
}

/** Whether two page-space rectangles are equal within 0.01 pt. */
export function sameRect(one: Rect, other: Rect): boolean {
  return (
    Math.abs(one.x0 - other.x0) < 0.01 &&
    Math.abs(one.y0 - other.y0) < 0.01 &&
    Math.abs(one.x1 - other.x1) < 0.01 &&
    Math.abs(one.y1 - other.y1) < 0.01
  );
}

/** Safe CSS `font-family` stack for `family`. */
export function cssFontFamily(family: string): string {
  const escaped = family.replace(/["\\]/g, "\\$&");
  return `"${escaped}", sans-serif`;
}

const PROBE_SAMPLE = "mmmmmmmmmmlli10W寬字體";
const renderableCache = new Map<string, boolean>();

/**
 * Whether WebView2 can render `font` directly while typing.
 *
 * Imported fonts are never handed to the frontend (ADR 0018), so WebView2 cannot render them and
 * always falls back to a system font while typing. For installed fonts, we measure sample text on
 * an offscreen canvas against generic fallbacks to detect registry/family name mismatches.
 */
export function isFontRenderable(font: FontInfo | undefined): boolean {
  if (!font) return false;
  if (font.source === "imported") return false;
  const key = `${font.family}|${font.bold ? 1 : 0}|${font.italic ? 1 : 0}`;
  const cached = renderableCache.get(key);
  if (cached !== undefined) return cached;

  let renderable = true;
  if (typeof document !== "undefined") {
    try {
      const canvas = document.createElement("canvas");
      const ctx = canvas.getContext("2d");
      if (ctx) {
        const prefix = `${font.italic ? "italic " : ""}${font.bold ? "700 " : "400 "}48px `;
        const escaped = font.family.replace(/["\\]/g, "\\$&");
        const measure = (stack: string) => {
          ctx.font = prefix + stack;
          return ctx.measureText(PROBE_SAMPLE).width;
        };
        const baseMono = measure("monospace");
        const baseSans = measure("sans-serif");
        const baseSerif = measure("serif");
        // In jsdom (no real text shaper), widths are 0: assume installed fonts are renderable.
        if (baseMono > 0 || baseSans > 0 || baseSerif > 0) {
          const withMono = measure(`"${escaped}", monospace`);
          const withSans = measure(`"${escaped}", sans-serif`);
          const withSerif = measure(`"${escaped}", serif`);
          renderable =
            Math.abs(withMono - baseMono) > 0.5 ||
            Math.abs(withSans - baseSans) > 0.5 ||
            Math.abs(withSerif - baseSerif) > 0.5;
        }
      }
    } catch {
      renderable = true;
    }
  }
  renderableCache.set(key, renderable);
  return renderable;
}

/** Minimum height (in points) for one line of text in a box. */
export function singleLineHeightPt(sizePt: number, lineStep = DEFAULT_LINE_STEP): number {
  const stepPt = (lineStep / 1000) * sizePt;
  return Math.max(LIMITS.minAnnotationSidePt, Math.ceil(stepPt + TEXT_BOX_PADDING_PT * 2 + 2));
}

/**
 * Initial page-space rectangle for a new inline text box placed at `at`.
 *
 * When `centered` is true (keyboard placement via Enter), the box starts centered around `at`;
 * when false (pointer click or context menu), its top-left starts at `at` (adjusted so it stays
 * inside the page).
 */
export function newInlineTextBoxRect(
  at: Point,
  page: PageSize,
  sizePt = DEFAULT_TEXT_SIZE,
  lineStep = DEFAULT_LINE_STEP,
  centered = false,
): Rect {
  const height = Math.min(singleLineHeightPt(sizePt, lineStep), page.heightPt);
  const availRight = Math.max(LIMITS.minAnnotationSidePt, page.widthPt - at.x - TEXT_BOX_EDGE_MARGIN_PT);
  const width = Math.min(
    INITIAL_INLINE_WIDTH_PT,
    centered ? page.widthPt : availRight,
    page.widthPt,
  );
  const raw: Rect = centered
    ? {
        x0: at.x - width / 2,
        y0: at.y - height / 2,
        x1: at.x + width / 2,
        y1: at.y + height / 2,
      }
    : {
        x0: at.x,
        y0: at.y - height / 2,
        x1: at.x + width,
        y1: at.y + height / 2,
      };
  return keptOnPage(raw, page);
}

/**
 * Grows `base` to fit `contentWidthPt` and `contentHeightPt` without leaving `page`.
 *
 * When `fixedWidth` is false (new box before user resizes width), the box widens rightward up to
 * the page's right edge (with margin), then wraps and grows downward. When `fixedWidth` is true
 * (editing an existing box, or after dragging width), width stays fixed and height grows downward.
 */
export function growInlineRect(
  base: Rect,
  contentWidthPt: number,
  contentHeightPt: number,
  page: PageSize,
  fixedWidth: boolean,
): Rect {
  const minSide = LIMITS.minAnnotationSidePt;
  const maxRight = Math.min(page.widthPt, Math.max(base.x1, page.widthPt - TEXT_BOX_EDGE_MARGIN_PT));
  const maxWidth = Math.max(minSide, maxRight - base.x0);
  const width = fixedWidth
    ? Math.max(minSide, Math.min(base.x1 - base.x0, page.widthPt - base.x0))
    : Math.min(maxWidth, Math.max(base.x1 - base.x0, contentWidthPt, minSide));
  const maxHeight = Math.max(minSide, page.heightPt - base.y0);
  const height = Math.min(maxHeight, Math.max(base.y1 - base.y0, contentHeightPt, minSide));
  return keptOnPage(
    {
      x0: base.x0,
      y0: base.y0,
      x1: base.x0 + width,
      y1: base.y0 + height,
    },
    page,
  );
}
