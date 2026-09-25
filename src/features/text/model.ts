// Selecting a page's text (MVP-15, docs/architecture/text-selection.md). Pure functions over the
// lines the worker reports: where a point falls, what a selection covers and the text it copies.
// Everything is in page space: PDF points, origin at the top left of the unrotated page, y down.

import type { PageText, Point, Quad, TextLine } from "@/ipc/generated/contract";

/**
 * A place between two characters: before character `index` of line `line` on page `page`
 * (all 0-based). An `index` equal to the line's length is after its last character.
 */
export type Caret = { page: number; line: number; index: number };

/** Where the selection started (`anchor`) and where it is now (`focus`); either may come first. */
export type TextSelection = { anchor: Caret; focus: Caret };

/** Where a point falls on a page's text: a line, and how far along it (points from its start). */
export type TextSpot = { page: number; line: number; along: number };

/** How close to a line (in points) a pointer must be to start selecting on it. */
export const TEXT_SLACK_PT = 2;

export function compareCarets(a: Caret, b: Caret): number {
  return a.page - b.page || a.line - b.line || a.index - b.index;
}

/** The selection's start and end in reading order. */
export function selectionRange(selection: TextSelection): [start: Caret, end: Caret] {
  return compareCarets(selection.anchor, selection.focus) <= 0
    ? [selection.anchor, selection.focus]
    : [selection.focus, selection.anchor];
}

export function hasSelectedText(selection: TextSelection | null): selection is TextSelection {
  return selection !== null && compareCarets(selection.anchor, selection.focus) !== 0;
}

export function sameSelection(a: TextSelection | null, b: TextSelection | null): boolean {
  if (a === null || b === null) return a === b;
  return compareCarets(a.anchor, b.anchor) === 0 && compareCarets(a.focus, b.focus) === 0;
}

const charCount = (line: TextLine) => line.edges.length - 1;
const lineEnd = (line: TextLine) => line.edges[line.edges.length - 1] ?? 0;

/** A line's own axes: unit vectors along its writing direction and across it, and its height. */
function axes(line: TextLine) {
  const { ul, ur, ll } = line.quad;
  const length = Math.hypot(ur.x - ul.x, ur.y - ul.y);
  const height = Math.hypot(ll.x - ul.x, ll.y - ul.y);
  const along = length > 0 ? { x: (ur.x - ul.x) / length, y: (ur.y - ul.y) / length } : { x: 1, y: 0 };
  const across = height > 0 ? { x: (ll.x - ul.x) / height, y: (ll.y - ul.y) / height } : { x: -along.y, y: along.x };
  return { along, across, height };
}

/**
 * How far `point` is from `line`, for choosing between lines: first by the distance across
 * lines (the line the pointer is level with wins), then along them (in a row of columns, the
 * nearer column). Also returns how far along the line the point is.
 */
function measure(line: TextLine, point: Point) {
  const { along, across, height } = axes(line);
  const dx = point.x - line.quad.ul.x;
  const dy = point.y - line.quad.ul.y;
  const t = dx * along.x + dy * along.y;
  const s = dx * across.x + dy * across.y;
  const offAlong = Math.max(-t, 0, t - lineEnd(line));
  const offAcross = Math.max(-s, 0, s - height);
  return { t, offAlong, offAcross };
}

/**
 * The spot on the page's text nearest to `point`. With a `slack`, only a line within that many
 * points counts (to start a selection on text); without one, the nearest line anywhere (to
 * extend a selection while dragging over margins and gaps).
 */
export function spotAt(text: PageText, page: number, point: Point, slack = Infinity): TextSpot | null {
  let best: TextSpot | null = null;
  let bestScore = Infinity;
  for (const [index, line] of text.lines.entries()) {
    const { t, offAlong, offAcross } = measure(line, point);
    if (offAlong > slack || offAcross > slack) continue;
    // Level with a line beats any distance along it.
    const score = offAcross * 1e6 + offAlong;
    if (score < bestScore) {
      best = { page, line: index, along: t };
      bestScore = score;
    }
  }
  return best;
}

/** The caret nearest to a spot: between the two characters whose middle the spot is closest to. */
export function caretAt(text: PageText, spot: TextSpot): Caret {
  const edges = text.lines[spot.line]?.edges ?? [0];
  let low = 0;
  let high = edges.length - 1;
  while (low < high) {
    const middle = (low + high) >> 1;
    if ((edges[middle]! + edges[middle + 1]!) / 2 < spot.along) low = middle + 1;
    else high = middle;
  }
  return { page: spot.page, line: spot.line, index: low };
}

/** The character a spot is on (the first or last one when it is beyond the line's ends). */
function charAt(line: TextLine, along: number): number {
  const edges = line.edges;
  let index = 0;
  while (index < charCount(line) - 1 && edges[index + 1]! <= along) index++;
  return index;
}

const wordSegmenter =
  typeof Intl !== "undefined" && "Segmenter" in Intl ? new Intl.Segmenter(undefined, { granularity: "word" }) : null;

/**
 * The word under a spot (double click), as a start and end caret. Words follow the platform's
 * word breaking, which also splits Chinese into words; a run of spaces or a punctuation mark is
 * a "word" of its own, as in a text editor.
 */
export function wordAt(text: PageText, spot: TextSpot): [Caret, Caret] {
  const line = text.lines[spot.line]!;
  const chars = Array.from(line.text);
  const target = charAt(line, spot.along);
  const caret = (index: number): Caret => ({ page: spot.page, line: spot.line, index });
  if (!wordSegmenter) {
    const isSpace = chars[target] === " ";
    let start = target;
    let end = target + 1;
    while (start > 0 && (chars[start - 1] === " ") === isSpace) start--;
    while (end < chars.length && (chars[end] === " ") === isSpace) end++;
    return [caret(start), caret(end)];
  }
  // Segments are indexed in UTF-16 code units; carets count characters.
  const unitStart: number[] = [];
  let units = 0;
  for (const char of chars) {
    unitStart.push(units);
    units += char.length;
  }
  const unit = unitStart[target]!;
  for (const segment of wordSegmenter.segment(line.text)) {
    const end = segment.index + segment.segment.length;
    if (unit >= segment.index && unit < end) {
      const endIndex = unitStart.indexOf(end);
      return [caret(unitStart.indexOf(segment.index)), caret(endIndex === -1 ? chars.length : endIndex)];
    }
  }
  return [caret(target), caret(target + 1)];
}

/** The whole line under a spot (triple click). */
export function lineAt(text: PageText, spot: TextSpot): [Caret, Caret] {
  const count = charCount(text.lines[spot.line]!);
  return [
    { page: spot.page, line: spot.line, index: 0 },
    { page: spot.page, line: spot.line, index: count },
  ];
}

/** How a drag selects: by characters from a caret, or by whole words or lines from a first one. */
export type DragStart =
  | { unit: "character"; anchor: Caret }
  | { unit: "word" | "line"; first: readonly [Caret, Caret] };

/**
 * The selection while dragging to `spot`. A drag that started with a double or triple click
 * extends by whole words or lines and always keeps the first one selected.
 */
export function dragSelection(start: DragStart, text: PageText, spot: TextSpot): TextSelection {
  if (start.unit === "character") return { anchor: start.anchor, focus: caretAt(text, spot) };
  const [from, to] = start.unit === "word" ? wordAt(text, spot) : lineAt(text, spot);
  const [firstStart, firstEnd] = start.first;
  if (compareCarets(from, firstStart) < 0) return { anchor: firstEnd, focus: from };
  return { anchor: firstStart, focus: compareCarets(to, firstEnd) > 0 ? to : firstEnd };
}

/** The selected characters of each line of `page` that has some: [line, from, to]. */
function selectedRuns(text: PageText, page: number, selection: TextSelection): [number, number, number][] {
  const [start, end] = selectionRange(selection);
  if (page < start.page || page > end.page) return [];
  const runs: [number, number, number][] = [];
  text.lines.forEach((line, index) => {
    const from = start.page === page && start.line === index ? start.index : 0;
    const to = end.page === page && end.line === index ? end.index : charCount(line);
    const afterStart = start.page < page || start.line <= index;
    const beforeEnd = end.page > page || end.line >= index;
    if (afterStart && beforeEnd && from < to) runs.push([index, from, Math.min(to, charCount(line))]);
  });
  return runs;
}

/** What to highlight on `page`: one quad per line, over its selected characters, in page space. */
export function selectionQuads(text: PageText, page: number, selection: TextSelection): Quad[] {
  return selectedRuns(text, page, selection).map(([index, from, to]) => {
    const line = text.lines[index]!;
    const { along } = axes(line);
    const { ul, ll } = line.quad;
    const at = (distance: number, base: Point) => ({ x: base.x + along.x * distance, y: base.y + along.y * distance });
    const start = line.edges[from]!;
    const end = line.edges[to]!;
    return { ul: at(start, ul), ur: at(end, ul), ll: at(start, ll), lr: at(end, ll) };
  });
}

/**
 * The selected text, a line break after every line but the last. `textOf` gives the text of each
 * page the selection spans; a page without it (it could not be read) adds nothing.
 */
export function selectedText(selection: TextSelection, textOf: (page: number) => PageText | undefined): string {
  const [start, end] = selectionRange(selection);
  const pieces: string[] = [];
  for (let page = start.page; page <= end.page; page++) {
    const text = textOf(page);
    if (!text) continue;
    for (const [index, from, to] of selectedRuns(text, page, selection)) {
      pieces.push(Array.from(text.lines[index]!.text).slice(from, to).join(""));
    }
  }
  return pieces.join("\n");
}
