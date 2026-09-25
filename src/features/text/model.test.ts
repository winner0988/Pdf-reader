import { describe, expect, it } from "vitest";

import {
  caretAt,
  dragSelection,
  hasSelectedText,
  lineAt,
  selectedText,
  selectionQuads,
  spotAt,
  TEXT_SLACK_PT,
  wordAt,
  type Caret,
  type TextSelection,
} from "@/features/text/model";
import type { PageText, TextLine } from "@/ipc/generated/contract";

/** A horizontal line of `text` from (x, y), every character `width` wide and 12 high. */
function line(text: string, x: number, y: number, width: number): TextLine {
  const count = Array.from(text).length;
  const end = x + count * width;
  return {
    text,
    quad: { ul: { x, y }, ur: { x: end, y }, ll: { x, y: y + 12 }, lr: { x: end, y: y + 12 } },
    edges: Array.from({ length: count + 1 }, (_, i) => i * width),
  };
}

const page = (...lines: TextLine[]): PageText => ({ lines, truncated: false });

// Page 0: an English line over a Chinese one. Page 1: one more line.
const first = page(line("Hello world", 72, 100, 6), line("中文字", 72, 120, 12));
const second = page(line("Page two", 72, 100, 6));
const pages = [first, second];

const caret = (pageIndex: number, lineIndex: number, index: number): Caret => ({
  page: pageIndex,
  line: lineIndex,
  index,
});
const select = (anchor: Caret, focus: Caret): TextSelection => ({ anchor, focus });

describe("hit-testing", () => {
  it("finds the caret between the characters nearest to a point on a line", () => {
    // "o" of "Hello" spans 96 to 102; its middle is at 99.
    const before = spotAt(first, 0, { x: 98, y: 105 }, TEXT_SLACK_PT)!;
    expect(before).toMatchObject({ page: 0, line: 0 });
    expect(caretAt(first, before)).toEqual(caret(0, 0, 4));
    expect(caretAt(first, spotAt(first, 0, { x: 100, y: 105 })!)).toEqual(caret(0, 0, 5));
    // Past the ends of a line: its first and last carets.
    expect(caretAt(first, spotAt(first, 0, { x: 10, y: 105 })!)).toEqual(caret(0, 0, 0));
    expect(caretAt(first, spotAt(first, 0, { x: 500, y: 125 })!)).toEqual(caret(0, 1, 3));
  });

  it("starts on text only close to a line, but follows the nearest line anywhere while dragging", () => {
    expect(spotAt(first, 0, { x: 300, y: 300 }, TEXT_SLACK_PT)).toBeNull();
    expect(spotAt(first, 0, { x: 72, y: 100 - TEXT_SLACK_PT - 1 }, TEXT_SLACK_PT)).toBeNull();
    expect(spotAt(first, 0, { x: 72, y: 100 - TEXT_SLACK_PT + 1 }, TEXT_SLACK_PT)).not.toBeNull();
    expect(spotAt(first, 0, { x: 300, y: 300 })).toMatchObject({ line: 1 });
    expect(spotAt(page(), 0, { x: 0, y: 0 })).toBeNull();
  });

  it("prefers the line level with the point, then the nearer column", () => {
    const columns = page(line("left", 72, 100, 6), line("right", 300, 100, 6), line("below", 72, 116, 6));
    expect(spotAt(columns, 0, { x: 250, y: 105 })).toMatchObject({ line: 1 });
    expect(spotAt(columns, 0, { x: 140, y: 105 })).toMatchObject({ line: 0 });
    // Right of a short line's end but level with it: that line, not a longer one nearby.
    expect(spotAt(columns, 0, { x: 200, y: 122 })).toMatchObject({ line: 2 });
  });
});

describe("words and lines", () => {
  it("selects the word, space or punctuation under a double click", () => {
    const spot = (x: number) => spotAt(first, 0, { x, y: 105 })!;
    expect(wordAt(first, spot(130))).toEqual([caret(0, 0, 6), caret(0, 0, 11)]);
    expect(wordAt(first, spot(73))).toEqual([caret(0, 0, 0), caret(0, 0, 5)]);
    expect(wordAt(first, spot(104))).toEqual([caret(0, 0, 5), caret(0, 0, 6)]);
  });

  it("splits Chinese into words without spaces", () => {
    const [start, end] = wordAt(first, spotAt(first, 0, { x: 90, y: 125 })!);
    // Whatever the word breaker makes of it, the clicked 文 is in the word, and the word is on its line.
    expect(start.line).toBe(1);
    expect(start.index).toBeLessThanOrEqual(1);
    expect(end.index).toBeGreaterThanOrEqual(2);
    expect(end.index).toBeLessThanOrEqual(3);
  });

  it("selects the whole line under a triple click", () => {
    expect(lineAt(first, spotAt(first, 0, { x: 80, y: 125 })!)).toEqual([caret(0, 1, 0), caret(0, 1, 3)]);
  });
});

describe("dragging", () => {
  it("selects characters from where the drag started, in either direction", () => {
    const start = { unit: "character", anchor: caret(0, 0, 2) } as const;
    expect(dragSelection(start, first, spotAt(first, 0, { x: 92, y: 125 })!)).toEqual(
      select(caret(0, 0, 2), caret(0, 1, 2)),
    );
    expect(dragSelection(start, first, spotAt(first, 0, { x: 72, y: 105 })!)).toEqual(
      select(caret(0, 0, 2), caret(0, 0, 0)),
    );
  });

  it("extends by whole words after a double click and keeps the first word", () => {
    const start = { unit: "word", first: [caret(0, 0, 6), caret(0, 0, 11)] } as const;
    // Back into "Hello": from its start to the end of "world".
    expect(dragSelection(start, first, spotAt(first, 0, { x: 80, y: 105 })!)).toEqual(
      select(caret(0, 0, 11), caret(0, 0, 0)),
    );
    // Within "world" itself: just the word.
    expect(dragSelection(start, first, spotAt(first, 0, { x: 110, y: 105 })!)).toEqual(
      select(caret(0, 0, 6), caret(0, 0, 11)),
    );
  });

  it("extends by whole lines after a triple click", () => {
    const start = { unit: "line", first: [caret(0, 0, 0), caret(0, 0, 11)] } as const;
    expect(dragSelection(start, first, spotAt(first, 0, { x: 75, y: 125 })!)).toEqual(
      select(caret(0, 0, 0), caret(0, 1, 3)),
    );
  });
});

describe("what a selection covers", () => {
  const selection = select(caret(0, 1, 2), caret(0, 0, 6));

  it("highlights the selected part of every line it spans", () => {
    expect(selectionQuads(first, 0, selection)).toEqual([
      { ul: { x: 108, y: 100 }, ur: { x: 138, y: 100 }, ll: { x: 108, y: 112 }, lr: { x: 138, y: 112 } },
      { ul: { x: 72, y: 120 }, ur: { x: 96, y: 120 }, ll: { x: 72, y: 132 }, lr: { x: 96, y: 132 } },
    ]);
    expect(selectionQuads(second, 1, selection)).toEqual([]);
  });

  it("follows a line written in another direction", () => {
    // Written downwards: characters 10 apart from y = 50, the line 10 wide to the left of x = 210.
    const vertical: TextLine = {
      text: "直書字",
      quad: { ul: { x: 210, y: 50 }, ur: { x: 210, y: 80 }, ll: { x: 200, y: 50 }, lr: { x: 200, y: 80 } },
      edges: [0, 10, 20, 30],
    };
    const text = page(vertical);
    const spot = spotAt(text, 0, { x: 205, y: 66 })!;
    expect(caretAt(text, spot)).toEqual(caret(0, 0, 2));
    expect(selectionQuads(text, 0, select(caret(0, 0, 1), caret(0, 0, 2)))).toEqual([
      { ul: { x: 210, y: 60 }, ur: { x: 210, y: 70 }, ll: { x: 200, y: 60 }, lr: { x: 200, y: 70 } },
    ]);
  });

  it("copies the selected text with a line break between lines and pages", () => {
    expect(selectedText(selection, (index) => pages[index])).toBe("world\n中文");
    const acrossPages = select(caret(0, 1, 1), caret(1, 0, 4));
    expect(selectedText(acrossPages, (index) => pages[index])).toBe("文字\nPage");
    // A page whose text could not be read adds nothing.
    expect(selectedText(acrossPages, (index) => (index === 1 ? undefined : pages[index]))).toBe("文字");
  });

  it("knows when nothing is selected", () => {
    expect(hasSelectedText(null)).toBe(false);
    expect(hasSelectedText(select(caret(0, 0, 3), caret(0, 0, 3)))).toBe(false);
    expect(hasSelectedText(selection)).toBe(true);
    // A selection ending at the start of a line copies nothing of that line.
    expect(selectedText(select(caret(0, 0, 6), caret(0, 1, 0)), (index) => pages[index])).toBe("world");
  });
});
