import { describe, expect, it } from "vitest";

import type { AnnotationSummary } from "@/ipc/generated/contract";
import { escapeMarkdown, formatAnnotationSummary } from "./markdown";

describe("escapeMarkdown", () => {
  it("escapes special characters *, _, #, `, [, ], <, >", () => {
    const input = "*bold* _italic_ #heading `code` [link] <tag>";
    const expected = "\\*bold\\* \\_italic\\_ \\#heading \\`code\\` \\[link\\] \\<tag\\>";
    expect(escapeMarkdown(input)).toBe(expected);
  });

  it("escapes line-leading -, +, and digits followed by a dot", () => {
    const input = "- item 1\n+ item 2\n1. first\n12. twelfth\nnormal - not line leading + not line leading 1. not line leading";
    const expected = "\\- item 1\n\\+ item 2\n1\\. first\n12\\. twelfth\nnormal - not line leading + not line leading 1. not line leading";
    expect(escapeMarkdown(input)).toBe(expected);
  });

  it("escapes line-leading list markers with leading whitespace", () => {
    const input = "  - indented item\n   + indented plus\n    5. indented number";
    const expected = "  \\- indented item\n   \\+ indented plus\n    5\\. indented number";
    expect(escapeMarkdown(input)).toBe(expected);
  });

  it("handles complex mixed markdown syntax", () => {
    const input = "# Title\n- [Link](url)\n2. `const x = 1 < 2 && 3 > 2;`\n_note_ *note*";
    const expected = "\\# Title\n\\- \\[Link\\](url)\n2\\. \\`const x = 1 \\< 2 && 3 \\> 2;\\`\n\\_note\\_ \\*note\\*";
    expect(escapeMarkdown(input)).toBe(expected);
  });
});

describe("formatAnnotationSummary", () => {
  const sampleSummary: AnnotationSummary = {
    items: [
      {
        pageIndex: 0,
        kind: "highlight",
        rect: { x0: 10, y0: 20, x1: 100, y1: 40 },
        text: "Sample *highlighted* text across\nlines",
      },
      {
        pageIndex: 0,
        kind: "note",
        rect: { x0: 50, y0: 60, x1: 70, y1: 80 },
        text: "- Note item 1\n1. Note item 2",
      },
      {
        pageIndex: 1,
        kind: "ink",
        rect: { x0: 0, y0: 0, x1: 50, y1: 50 },
        text: "第 2 頁有手繪",
      },
      {
        pageIndex: 1,
        kind: "stamp",
        rect: { x0: 100, y0: 100, x1: 200, y1: 200 },
        text: "第 2 頁有印章",
      },
      {
        pageIndex: 2,
        kind: "textBox",
        rect: { x0: 10, y0: 10, x1: 80, y1: 80 },
        text: "Text box content",
      },
    ],
    truncated: false,
  };

  it("formats as plain text without escaping", () => {
    const result = formatAnnotationSummary(sampleSummary, {
      format: "text",
      includeTitle: true,
      docTitle: "sample.pdf",
    });
    expect(result).toContain("sample.pdf\n\n");
    expect(result).toContain("第 1 頁 [螢光筆]\nSample *highlighted* text across\nlines");
    expect(result).toContain("第 1 頁 [附註]\n- Note item 1\n1. Note item 2");
    expect(result).toContain("第 2 頁 [手繪]\n第 2 頁有手繪");
    expect(result).toContain("第 2 頁 [印章]\n第 2 頁有印章");
    expect(result).toContain("第 3 頁 [文字方塊]\nText box content");
  });

  it("formats as Markdown with proper escaping and headings", () => {
    const result = formatAnnotationSummary(sampleSummary, {
      format: "markdown",
      includeTitle: true,
      docTitle: "my_doc.pdf",
    });
    expect(result).toContain("# my\\_doc.pdf\n\n");
    expect(result).toContain("## 第 1 頁 [螢光筆]\n\nSample \\*highlighted\\* text across\nlines");
    expect(result).toContain("## 第 1 頁 [附註]\n\n\\- Note item 1\n1\\. Note item 2");
  });

  it("omits title when includeTitle is false", () => {
    const result = formatAnnotationSummary(sampleSummary, {
      format: "text",
      includeTitle: false,
      docTitle: "sample.pdf",
    });
    expect(result.startsWith("第 1 頁 [螢光筆]")).toBe(true);
    expect(result).not.toContain("sample.pdf");
  });

  it("handles empty annotation summary", () => {
    const emptySummary: AnnotationSummary = { items: [], truncated: false };
    const md = formatAnnotationSummary(emptySummary, { format: "markdown" });
    expect(md).toContain("*（此文件沒有註解）*");

    const text = formatAnnotationSummary(emptySummary, { format: "text" });
    expect(text).toContain("（此文件沒有註解）");
  });

  it("appends truncation indicator when truncated is true", () => {
    const truncatedSummary: AnnotationSummary = {
      items: [
        {
          pageIndex: 0,
          kind: "note",
          rect: { x0: 0, y0: 0, x1: 10, y1: 10 },
          text: "Note",
        },
      ],
      truncated: true,
    };
    const md = formatAnnotationSummary(truncatedSummary, { format: "markdown" });
    expect(md).toContain("---\n*已達大小上限，部分註解未列出*");

    const text = formatAnnotationSummary(truncatedSummary, { format: "text" });
    expect(text).toContain("---\n已達大小上限，部分註解未列出");
  });
});
