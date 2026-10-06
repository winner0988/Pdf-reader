// What the page shows of annotations (B2-07, docs/architecture/annotations.md): the highlighter's
// colors, how an annotation is named, which one is under the pointer, and what a note may say.

import { strings } from "@/i18n/zh-TW";
import { LIMITS, type HighlightColor, type PageAnnotation } from "@/ipc/generated/contract";

const t = strings.annotations;

export const HIGHLIGHT_COLORS: HighlightColor[] = ["yellow", "green", "blue", "pink"];

/** The swatch of each highlighter color (the worker's RGB, docs/architecture/annotations.md). */
export const SWATCH: Record<HighlightColor, string> = {
  yellow: "rgb(255 235 0)",
  green: "rgb(140 230 89)",
  blue: "rgb(115 191 255)",
  pink: "rgb(255 140 191)",
};

/** How an annotation is read out and named in its toolbar. */
export function annotationLabel(annotation: PageAnnotation): string {
  switch (annotation.kind) {
    case "highlight":
      return annotation.color ? t.highlightIn(t.colors[annotation.color]) : t.kind.highlight;
    case "note":
      return annotation.text ? t.noteSaying(annotation.text) : t.kind.note;
    case "other":
      return t.kind.other;
  }
}

/** The annotation of `annotations` under `point` (page space): the last drawn, which is on top. */
export function annotationAt(
  annotations: readonly PageAnnotation[],
  point: { x: number; y: number },
): PageAnnotation | null {
  for (let index = annotations.length - 1; index >= 0; index--) {
    const { rect } = annotations[index]!;
    if (point.x >= rect.x0 && point.x <= rect.x1 && point.y >= rect.y0 && point.y <= rect.y1) {
      return annotations[index]!;
    }
  }
  return null;
}

/**
 * What the main process refuses in a note (crates/ipc_contract/src/text.rs), as code point ranges:
 * control characters other than the line break, and characters that change how text is shown
 * without being visible (bidirectional controls, zero-width characters, the byte order mark).
 */
const NOT_IN_A_NOTE: readonly (readonly [from: number, to: number])[] = [
  [0x0000, 0x0009],
  [0x000b, 0x001f],
  [0x007f, 0x009f],
  [0x061c, 0x061c],
  [0x200b, 0x200f],
  [0x202a, 0x202e],
  [0x2060, 0x2069],
  [0xfeff, 0xfeff],
];

const refused = (codePoint: number) => NOT_IN_A_NOTE.some(([from, to]) => codePoint >= from && codePoint <= to);

/**
 * A note's text as typed, made into what the main process accepts (B2-07): line breaks are LF, a
 * tab is four spaces, control and invisible formatting characters are dropped, and the ends are
 * trimmed. The user's own spacing inside stays.
 */
export function noteText(typed: string): string {
  const unified = typed.replace(/\r\n?/g, "\n").replace(/\t/g, "    ");
  let kept = "";
  for (const character of unified) {
    if (!refused(character.codePointAt(0) ?? 0)) kept += character;
  }
  return kept.trim();
}

/** Whether `text` is longer than a note may be (in UTF-8 bytes, as the main process counts). */
export function noteTooLong(text: string): boolean {
  return new TextEncoder().encode(text).length > LIMITS.maxNoteTextBytes;
}
