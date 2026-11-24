import { describe, expect, it } from "vitest";

import {
  buildPageUtterance,
  buildSelectionUtterance,
  clampSpeechRate,
  highlightForUtf16Range,
  resolveVoiceId,
} from "@/features/readAloud/model";
import type { PageText, SpeechVoice, TextLine } from "@/ipc/generated/contract";

function makeLine(text: string, y: number, charWidth = 10): TextLine {
  const chars = Array.from(text);
  const edges = chars.map((_, i) => i * charWidth);
  edges.push(chars.length * charWidth);
  const width = chars.length * charWidth;
  return {
    text,
    quad: {
      ul: { x: 0, y },
      ur: { x: width, y },
      ll: { x: 0, y: y + 12 },
      lr: { x: width, y: y + 12 },
    },
    edges,
  };
}

function makePage(lines: TextLine[]): PageText {
  return { lines, truncated: false, recognised: false };
}

describe("readAloud model", () => {
  it("maps mixed Chinese, English, and line breaks to exact line quads", () => {
    const page = makePage([makeLine("你好 world", 10), makeLine("第二行", 30)]);
    const utterance = buildPageUtterance(0, page);
    expect(utterance).not.toBeNull();
    expect(utterance!.text).toBe("你好 world\n第二行");

    // "world" is at UTF-16 offset 3, length 5 on line 0 (chars 3..8 -> edges 30..80).
    const worldHits = highlightForUtf16Range(utterance!, 3, 5);
    expect(worldHits).toHaveLength(1);
    expect(worldHits[0]!.pageIndex).toBe(0);
    expect(worldHits[0]!.quads).toEqual([
      {
        ul: { x: 30, y: 10 },
        ur: { x: 80, y: 10 },
        ll: { x: 30, y: 22 },
        lr: { x: 80, y: 22 },
      },
    ]);

    // "第二行" starts after "\n" (UTF-16 offset 9, length 3 -> line 1, chars 0..3 -> edges 0..30).
    const line2Hits = highlightForUtf16Range(utterance!, 9, 3);
    expect(line2Hits).toHaveLength(1);
    expect(line2Hits[0]!.quads).toEqual([
      {
        ul: { x: 0, y: 30 },
        ur: { x: 30, y: 30 },
        ll: { x: 0, y: 42 },
        lr: { x: 30, y: 42 },
      },
    ]);
  });

  it("maps UTF-16 offsets accurately across non-BMP surrogate pairs (U+20000)", () => {
    // Unicode scalars (indices for edges):
    // 0:你 1:好 2:' ' 3:w 4:o 5:r 6:l 7:d 8:' ' 9:𠀀 10:' ' 11:t 12:e 13:s 14:t (15 scalars)
    // UTF-16 code units:
    // 0:你 1:好 2:' ' 3..7:world 8:' ' 9..10:𠀀(2 units) 11:' ' 12..15:test (16 UTF-16 units)
    const lineText = "你好 world \u{20000} test";
    const page = makePage([makeLine(lineText, 20, 10)]);
    const utterance = buildPageUtterance(2, page)!;
    expect(utterance.text.length).toBe(16);

    // Highlight the non-BMP character itself (UTF-16 9..11 -> scalar index 9 -> edges 90..100).
    const nonBmpHits = highlightForUtf16Range(utterance, 9, 2);
    expect(nonBmpHits).toEqual([
      {
        pageIndex: 2,
        quads: [
          {
            ul: { x: 90, y: 20 },
            ur: { x: 100, y: 20 },
            ll: { x: 90, y: 32 },
            lr: { x: 100, y: 32 },
          },
        ],
      },
    ]);

    // Highlight "test" after the non-BMP character: UTF-16 start=12, len=4 -> scalar 11..15 -> edges 110..150.
    // A buggy implementation using UTF-16 indices directly as char indices would look up edges 120..160 (out of bounds / shifted!).
    const testHits = highlightForUtf16Range(utterance, 12, 4);
    expect(testHits).toEqual([
      {
        pageIndex: 2,
        quads: [
          {
            ul: { x: 110, y: 20 },
            ur: { x: 150, y: 20 },
            ll: { x: 110, y: 32 },
            lr: { x: 150, y: 32 },
          },
        ],
      },
    ]);
  });

  it("builds multi-line and multi-page selection utterances with exact highlight offsets", () => {
    const pages = new Map<number, PageText>([
      [0, makePage([makeLine("Alpha Beta", 10), makeLine("Gamma Delta", 30)])],
      [1, makePage([makeLine("Epsilon Zeta", 10)])],
    ]);

    // Select from "Beta" on page 0 line 0 (char 6) to "Epsilon" on page 1 line 0 (char 7).
    const selection = {
      anchor: { page: 0, line: 0, index: 6 },
      focus: { page: 1, line: 0, index: 7 },
    };
    const utterance = buildSelectionUtterance(selection, (p) => pages.get(p));
    expect(utterance).not.toBeNull();
    expect(utterance!.mode).toBe("selection");
    expect(utterance!.text).toBe("Beta\nGamma Delta\nEpsilon");

    // "Beta" -> UTF-16 0..4 -> page 0 line 0 chars 6..10 -> x: 60..100
    expect(highlightForUtf16Range(utterance!, 0, 4)).toEqual([
      {
        pageIndex: 0,
        quads: [{ ul: { x: 60, y: 10 }, ur: { x: 100, y: 10 }, ll: { x: 60, y: 22 }, lr: { x: 100, y: 22 } }],
      },
    ]);

    // "Delta" -> UTF-16 11..16 -> page 0 line 1 chars 6..11 -> x: 60..110
    expect(highlightForUtf16Range(utterance!, 11, 5)).toEqual([
      {
        pageIndex: 0,
        quads: [{ ul: { x: 60, y: 30 }, ur: { x: 110, y: 30 }, ll: { x: 60, y: 42 }, lr: { x: 110, y: 42 } }],
      },
    ]);

    // "Epsilon" -> UTF-16 17..24 -> page 1 line 0 chars 0..7 -> x: 0..70
    expect(highlightForUtf16Range(utterance!, 17, 7)).toEqual([
      {
        pageIndex: 1,
        quads: [{ ul: { x: 0, y: 10 }, ur: { x: 70, y: 10 }, ll: { x: 0, y: 22 }, lr: { x: 70, y: 22 } }],
      },
    ]);
  });

  it("returns null for pages with no speakable text", () => {
    expect(buildPageUtterance(0, makePage([]))).toBeNull();
    expect(buildPageUtterance(0, makePage([makeLine("   ", 0)]))).toBeNull();
  });

  it("truncates safely at UTF-8 byte boundary without splitting characters", () => {
    const page = makePage([makeLine("你好世界", 0)]); // 4 chars * 3 UTF-8 bytes = 12 bytes
    const utterance = buildPageUtterance(0, page, 7)!;
    expect(utterance.text).toBe("你好");
    expect(utterance.map).toHaveLength(2);
  });

  it("falls back to default voice when saved voice is missing", () => {
    const voices: SpeechVoice[] = [
      { id: "v1", name: "David", language: "en-US", isDefault: false },
      { id: "v2", name: "Hanhan", language: "zh-TW", isDefault: true },
    ];
    expect(resolveVoiceId("v1", voices)).toBe("v1");
    expect(resolveVoiceId("uninstalled-voice", voices)).toBe("v2");
    expect(resolveVoiceId(null, voices)).toBe("v2");
    expect(resolveVoiceId("v1", [])).toBeNull();
  });

  it("clamps speaking rates to Windows' valid bounds", () => {
    expect(clampSpeechRate(1.0)).toBe(1.0);
    expect(clampSpeechRate(0.1)).toBe(0.5);
    expect(clampSpeechRate(10)).toBe(6.0);
    expect(clampSpeechRate(Number.NaN)).toBe(1.0);
  });
});
