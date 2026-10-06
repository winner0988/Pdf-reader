import { describe, expect, it } from "vitest";

import { annotationAt, annotationLabel, noteText, noteTooLong } from "@/features/annotations/model";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type PageAnnotation } from "@/ipc/generated/contract";

const highlight: PageAnnotation = {
  id: 4,
  kind: "highlight",
  rect: { x0: 70, y0: 70, x1: 200, y1: 92 },
  color: "yellow",
  text: null,
};
const note: PageAnnotation = {
  id: 5,
  kind: "note",
  rect: { x0: 150, y0: 80, x1: 170, y1: 100 },
  color: null,
  text: "Existing note",
};

/** Characters by code point: the invisible ones are not written out in the source. */
const chars = (...codes: number[]) => String.fromCodePoint(...codes);

describe("what a note may say", () => {
  it("is what the main process accepts: LF line breaks, no control or invisible characters", () => {
    const bell = chars(0x07);
    const override = chars(0x202e);
    const zeroWidth = chars(0x200b);
    expect(noteText("  第一行\r\n  second\rthird\t4 " + bell + override + "!" + zeroWidth + "  ")).toBe(
      "第一行\n  second\nthird    4 !",
    );
    // C1 controls and the byte order mark go too.
    expect(noteText("a" + chars(0x85) + "b" + chars(0x9f) + "c" + chars(0xfeff) + "d")).toBe("abcd");
    expect(noteText(" \n\t ")).toBe("");
    // The user's own spacing inside stays.
    expect(noteText("one  two\n\n  three")).toBe("one  two\n\n  three");
  });

  it("is counted in UTF-8 bytes, as the main process counts", () => {
    expect(noteTooLong("字".repeat(Math.floor(LIMITS.maxNoteTextBytes / 3)))).toBe(false);
    expect(noteTooLong("字".repeat(Math.floor(LIMITS.maxNoteTextBytes / 3) + 1))).toBe(true);
    expect(noteTooLong("a".repeat(LIMITS.maxNoteTextBytes))).toBe(false);
    expect(noteTooLong("a".repeat(LIMITS.maxNoteTextBytes + 1))).toBe(true);
  });
});

describe("annotations on a page", () => {
  it("finds the one on top at a point", () => {
    expect(annotationAt([highlight, note], { x: 160, y: 90 })).toBe(note);
    expect(annotationAt([highlight, note], { x: 100, y: 90 })).toBe(highlight);
    expect(annotationAt([highlight, note], { x: 300, y: 300 })).toBeNull();
    expect(annotationAt([], { x: 100, y: 90 })).toBeNull();
  });

  it("is named for what it is, for the screen reader and the toolbar", () => {
    expect(annotationLabel(highlight)).toBe(strings.annotations.highlightIn(strings.annotations.colors.yellow));
    expect(annotationLabel({ ...highlight, color: null })).toBe(strings.annotations.kind.highlight);
    expect(annotationLabel(note)).toBe(strings.annotations.noteSaying("Existing note"));
    expect(annotationLabel({ ...note, text: null })).toBe(strings.annotations.kind.note);
    expect(annotationLabel({ ...note, kind: "other", text: null })).toBe(strings.annotations.kind.other);
  });
});
