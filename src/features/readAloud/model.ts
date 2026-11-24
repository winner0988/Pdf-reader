// Pure text assembly and UTF-16 -> page/line/character mapping for Read Aloud (B6-14, #599).
//
// Windows WinRT `SpeechCue` reports `StartPositionInInput` and `EndPositionInInput` in UTF-16
// code units, whereas `PageText`'s `TextLine.edges` is indexed by Unicode scalar values (code
// points). Characters outside the BMP (e.g. U+20000, emoji) occupy 2 UTF-16 code units but 1
// entry in `edges`. The lookup table built here maps every UTF-16 code unit index in the spoken
// string to its exact `(page, line, charIndex)` so spoken-word highlights stay aligned even
// across non-BMP characters, mixed CJK/Latin text, and multi-line selections.

import { selectionRange, type TextSelection } from "@/features/text/model";
import type { Highlight } from "@/features/viewer/highlights";
import { LIMITS, type PageText, type Point, type Quad, type SpeechVoice, type TextLine } from "@/ipc/generated/contract";

/** Preset speaking rates within Windows' accepted range (0.5–6.0). */
export const SPEECH_RATES = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3] as const;

/**
 * Maximum UTF-16 code units sent in a single `speak_text` call.
 * Worst-case UTF-8 size is 6,000 bytes (3 bytes/unit for BMP CJK), staying well within
 * `LIMITS.maxSpeechTextBytes` (16 KiB) while keeping in-memory synthesis latency and
 * cancellation latency short.
 */
export const MAX_SPEECH_CHUNK_UTF16 = 2_000;

/**
 * Maximum UTF-8 bytes assembled into one page or selection utterance before chunking.
 * Covers a full page at `LIMITS.maxPageTextChars` (100,000 scalars <= 400,000 UTF-8 bytes)
 * while bounding frontend memory for large multi-page selections.
 */
export const MAX_UTTERANCE_BYTES = 512 * 1024;

export type SpeechChunk = {
  /** Inclusive start offset in the utterance text (UTF-16 code units). */
  start: number;
  /** Exclusive end offset in the utterance text (UTF-16 code units). */
  end: number;
  /** Slice `text.slice(start, end)` sent to `speak_text`. */
  text: string;
};

export type Utf16Position = {
  page: number;
  line: number;
  /** 0-based Unicode scalar (code point) index within `TextLine.text` (for `TextLine.edges`). */
  charIndex: number;
};

export type SpeechUtterance = {
  /** Plain text sent to the speech engine. */
  text: string;
  /**
   * Maps each UTF-16 code unit index `0..text.length` to its source character in `pages`,
   * or `null` for inserted line breaks (`"\n"`).
   */
  map: (Utf16Position | null)[];
  /** The `PageText` snapshots referenced by `map`, keyed by 0-based page index. */
  pages: ReadonlyMap<number, PageText>;
  mode: "page" | "selection";
  /** 0-based page index for `"page"` mode, or first page of `"selection"` mode. */
  pageIndex: number;
};

const utf8Encoder = new TextEncoder();

function charCount(line: TextLine): number {
  return Math.max(0, line.edges.length - 1);
}

function lineQuadForRange(line: TextLine, fromChar: number, toChar: number): Quad | null {
  const maxChar = charCount(line);
  const start = Math.max(0, Math.min(fromChar, maxChar));
  const end = Math.max(start, Math.min(toChar, maxChar));
  if (start >= end) return null;
  const startEdge = line.edges[start];
  const endEdge = line.edges[end];
  if (startEdge === undefined || endEdge === undefined) return null;

  const { ul, ur, ll } = line.quad;
  const length = Math.hypot(ur.x - ul.x, ur.y - ul.y);
  const along = length > 0 ? { x: (ur.x - ul.x) / length, y: (ur.y - ul.y) / length } : { x: 1, y: 0 };
  const at = (distance: number, base: Point): Point => ({
    x: base.x + along.x * distance,
    y: base.y + along.y * distance,
  });
  return {
    ul: at(startEdge, ul),
    ur: at(endEdge, ul),
    ll: at(startEdge, ll),
    lr: at(endEdge, ll),
  };
}

type LineRun = { page: number; line: number; fromChar: number; toChar: number; text: string };

function buildUtteranceFromRuns(
  runs: LineRun[],
  pages: Map<number, PageText>,
  mode: "page" | "selection",
  pageIndex: number,
  maxBytes: number,
): SpeechUtterance | null {
  const pieces: string[] = [];
  const map: (Utf16Position | null)[] = [];
  let bytes = 0;

  for (const run of runs) {
    const chars = Array.from(run.text).slice(run.fromChar, run.toChar);
    if (chars.length === 0) continue;

    if (pieces.length > 0) {
      if (bytes + 1 > maxBytes) break;
      pieces.push("\n");
      map.push(null);
      bytes += 1;
    }

    let linePiece = "";
    let charIndex = run.fromChar;
    let truncated = false;
    for (const ch of chars) {
      const chBytes = utf8Encoder.encode(ch).length;
      if (bytes + chBytes > maxBytes) {
        truncated = true;
        break;
      }
      linePiece += ch;
      bytes += chBytes;
      // Each Unicode scalar occupies `ch.length` UTF-16 code units (1 for BMP, 2 for non-BMP).
      for (let u = 0; u < ch.length; u++) {
        map.push({ page: run.page, line: run.line, charIndex });
      }
      charIndex++;
    }
    if (linePiece.length > 0) {
      pieces.push(linePiece);
    }
    if (truncated) break;
  }

  const text = pieces.join("");
  if (text.trim().length === 0) return null;
  return { text, map, pages, mode, pageIndex };
}

/** Builds a speakable utterance and UTF-16 position map for a single page (0-based `pageIndex`). */
export function buildPageUtterance(
  pageIndex: number,
  pageText: PageText,
  maxBytes: number = MAX_UTTERANCE_BYTES,
): SpeechUtterance | null {
  const runs: LineRun[] = [];
  pageText.lines.forEach((line, lineIndex) => {
    const count = charCount(line);
    if (count > 0) {
      runs.push({ page: pageIndex, line: lineIndex, fromChar: 0, toChar: count, text: line.text });
    }
  });
  const pages = new Map<number, PageText>([[pageIndex, pageText]]);
  return buildUtteranceFromRuns(runs, pages, "page", pageIndex, maxBytes);
}

/** Builds a speakable utterance and UTF-16 position map for a multi-line or multi-page selection. */
export function buildSelectionUtterance(
  selection: TextSelection,
  textOf: (page: number) => PageText | undefined,
  maxBytes: number = MAX_UTTERANCE_BYTES,
): SpeechUtterance | null {
  const [start, end] = selectionRange(selection);
  const runs: LineRun[] = [];
  const pages = new Map<number, PageText>();

  for (let page = start.page; page <= end.page; page++) {
    const pageText = textOf(page);
    if (!pageText) continue;
    pages.set(page, pageText);
    pageText.lines.forEach((line, lineIndex) => {
      const count = charCount(line);
      const from = start.page === page && start.line === lineIndex ? start.index : 0;
      const to = end.page === page && end.line === lineIndex ? end.index : count;
      const afterStart = start.page < page || start.line <= lineIndex;
      const beforeEnd = end.page > page || end.line >= lineIndex;
      const clampedTo = Math.min(to, count);
      if (afterStart && beforeEnd && from < clampedTo) {
        runs.push({ page, line: lineIndex, fromChar: from, toChar: clampedTo, text: line.text });
      }
    });
  }

  return buildUtteranceFromRuns(runs, pages, "selection", start.page, maxBytes);
}

/**
 * Converts a UTF-16 word boundary range from `SpeechEvent::Word` into page-space highlight
 * quads (grouped by page). Returns an empty list if the range falls outside mapped characters.
 */
export function highlightForUtf16Range(
  utterance: SpeechUtterance,
  utf16Start: number,
  utf16Len: number,
): Highlight[] {
  if (utf16Len <= 0 || utf16Start < 0) return [];
  const endUnit = Math.min(utterance.map.length, utf16Start + utf16Len);
  if (utf16Start >= endUnit) return [];

  // Track min..max charIndex per (page, line).
  const spans = new Map<string, { page: number; line: number; minChar: number; maxChar: number }>();
  for (let u = utf16Start; u < endUnit; u++) {
    const pos = utterance.map[u];
    if (!pos) continue;
    const key = `${pos.page}:${pos.line}`;
    const existing = spans.get(key);
    if (!existing) {
      spans.set(key, { page: pos.page, line: pos.line, minChar: pos.charIndex, maxChar: pos.charIndex });
    } else {
      if (pos.charIndex < existing.minChar) existing.minChar = pos.charIndex;
      if (pos.charIndex > existing.maxChar) existing.maxChar = pos.charIndex;
    }
  }

  const byPage = new Map<number, Quad[]>();
  for (const span of spans.values()) {
    const pageText = utterance.pages.get(span.page);
    const line = pageText?.lines[span.line];
    if (!line) continue;
    const quad = lineQuadForRange(line, span.minChar, span.maxChar + 1);
    if (!quad) continue;
    const list = byPage.get(span.page);
    if (list) list.push(quad);
    else byPage.set(span.page, [quad]);
  }

  return [...byPage.entries()]
    .sort(([a], [b]) => a - b)
    .map(([pageIndex, quads]) => ({ pageIndex, quads }));
}

/**
 * Picks the effective voice id from the installed list: keeps `savedVoiceId` when it is still
 * installed, otherwise falls back to Windows' default voice (or the first listed voice), or
 * `null` when no voices are installed.
 */
export function resolveVoiceId(savedVoiceId: string | null, voices: readonly SpeechVoice[]): string | null {
  if (voices.length === 0) return null;
  if (savedVoiceId !== null && voices.some((v) => v.id === savedVoiceId)) {
    return savedVoiceId;
  }
  return voices.find((v) => v.isDefault)?.id ?? voices[0]!.id;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function isWhitespace(ch: string): boolean {
  return /\s/.test(ch);
}

function isClosingPunctCode(code: number): boolean {
  return (
    code === 0x0022 || // "
    code === 0x0027 || // '
    code === 0x0029 || // )
    code === 0x005d || // ]
    code === 0x2019 || // RIGHT SINGLE QUOTATION MARK
    code === 0x201d || // RIGHT DOUBLE QUOTATION MARK
    code === 0x300b || // RIGHT DOUBLE ANGLE BRACKET
    code === 0x300d || // RIGHT CORNER BRACKET
    code === 0x300f || // RIGHT WHITE CORNER BRACKET
    code === 0x3011 || // RIGHT BLACK LENTICULAR BRACKET
    code === 0xff09 // FULLWIDTH RIGHT PARENTHESIS
  );
}

function isCjkSentenceEndCode(code: number): boolean {
  return (
    code === 0x3002 || // IDEOGRAPHIC FULL STOP
    code === 0xff01 || // FULLWIDTH EXCLAMATION MARK
    code === 0xff1f || // FULLWIDTH QUESTION MARK
    code === 0xff1b // FULLWIDTH SEMICOLON
  );
}

function isAsciiSentenceEndCode(code: number): boolean {
  return code === 0x002e || code === 0x0021 || code === 0x003f || code === 0x003b;
}

/**
 * Picks the next speakable chunk starting at or after `fromUtf16`, capped at `maxUtf16`
 * UTF-16 code units.
 *
 * - Never starts or ends inside a UTF-16 surrogate pair.
 * - Skips leading whitespace so `speak_text` is never given whitespace-only text, while keeping
 *   `start` aligned with `text` so `baseUtf16 + event.utf16Start` maps back to page coordinates.
 * - When remaining text exceeds `maxUtf16`, prefers breaking at a paragraph break (`\n\n`), then
 *   a sentence boundary (CJK full-stop/exclamation/question/semicolon or ASCII `.!?;` followed by
 *   whitespace/end), then a line break (`\n`), then whitespace, and falls back to a surrogate-safe
 *   hard cut at `maxUtf16`.
 */
export function nextSpeechChunk(
  text: string,
  fromUtf16: number,
  maxUtf16: number = MAX_SPEECH_CHUNK_UTF16,
): SpeechChunk | null {
  let start = Math.max(0, Math.min(Math.floor(fromUtf16), text.length));
  if (
    start > 0 &&
    start < text.length &&
    isLowSurrogate(text.charCodeAt(start)) &&
    isHighSurrogate(text.charCodeAt(start - 1))
  ) {
    start -= 1;
  }

  while (start < text.length && isWhitespace(text[start]!)) {
    start += 1;
  }
  if (start >= text.length) return null;

  const limit = Math.max(2, Math.floor(maxUtf16));
  if (text.length - start <= limit) {
    return { start, end: text.length, text: text.slice(start) };
  }

  const hardEnd = start + limit;
  const preferredMin = start + Math.floor(limit / 2);

  // 1. Prefer a paragraph break (`\n\n`) in the second half of the window.
  for (let i = hardEnd - 2; i >= preferredMin; i--) {
    if (text[i] === "\n" && text[i + 1] === "\n") {
      let end = i;
      while (end > start && isWhitespace(text[end - 1]!)) end -= 1;
      if (end > start) {
        return { start, end, text: text.slice(start, end) };
      }
    }
  }

  // 2. Sentence boundary anywhere in `(start, hardEnd]`.
  for (let i = hardEnd - 1; i >= start; i--) {
    const code = text.charCodeAt(i);
    if (isCjkSentenceEndCode(code)) {
      let cut = i + 1;
      while (cut < hardEnd && isClosingPunctCode(text.charCodeAt(cut))) cut += 1;
      if (cut > start) {
        return { start, end: cut, text: text.slice(start, cut) };
      }
    } else if (isAsciiSentenceEndCode(code)) {
      let cut = i + 1;
      while (cut < hardEnd && isClosingPunctCode(text.charCodeAt(cut))) cut += 1;
      if (cut > start && (cut === text.length || isWhitespace(text[cut]!))) {
        return { start, end: cut, text: text.slice(start, cut) };
      }
    }
  }

  // 3. Line break (`\n`) anywhere in `(start, hardEnd)`.
  for (let i = hardEnd - 1; i > start; i--) {
    if (text[i] === "\n") {
      let end = i;
      while (end > start && isWhitespace(text[end - 1]!)) end -= 1;
      if (end > start) {
        return { start, end, text: text.slice(start, end) };
      }
    }
  }

  // 4. Word/whitespace boundary anywhere in `(start, hardEnd)`.
  for (let i = hardEnd - 1; i > start; i--) {
    if (isWhitespace(text[i]!)) {
      let end = i;
      while (end > start && isWhitespace(text[end - 1]!)) end -= 1;
      if (end > start) {
        return { start, end, text: text.slice(start, end) };
      }
    }
  }

  // 5. Hard cut at `hardEnd`, backing up 1 unit if it would split a UTF-16 surrogate pair.
  let end = hardEnd;
  if (
    end < text.length &&
    isHighSurrogate(text.charCodeAt(end - 1)) &&
    isLowSurrogate(text.charCodeAt(end))
  ) {
    end -= 1;
  }
  return { start, end, text: text.slice(start, end) };
}

/** Clamps a speaking rate into the supported range. */
export function clampSpeechRate(rate: number): number {
  if (!Number.isFinite(rate)) return 1.0;
  return Math.min(LIMITS.maxSpeechRate, Math.max(LIMITS.minSpeechRate, rate));
}
