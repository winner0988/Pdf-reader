// Annotation summary formatting and Markdown escaping (Issue #306).
import type { AnnotationSummary, AnnotationSummaryFormat } from "@/ipc/generated/contract";
import { strings } from "@/i18n/zh-TW";

/**
 * Escapes Markdown special characters: *, _, #, `, [, ], <, >,
 * and line-leading -, +, or digits followed by a period.
 */
export function escapeMarkdown(text: string): string {
  // First escape *, _, #, `, [, ], <, >
  let escaped = text.replace(/[*_#`[\]<>]/g, (match) => `\\${match}`);
  // Then escape line-leading -, +, or digits followed by a period (even with leading whitespace)
  escaped = escaped.replace(/^([ \t]*)([-+])/gm, "$1\\$2");
  escaped = escaped.replace(/^([ \t]*)(\d+)\./gm, "$1$2\\.");
  return escaped;
}

export type FormatSummaryOptions = {
  format: AnnotationSummaryFormat;
  includeTitle?: boolean;
  docTitle?: string;
};

function kindName(kind: string): string {
  switch (kind) {
    case "highlight":
      return strings.annotations.highlight;
    case "note":
      return strings.annotations.kind.note;
    case "ink":
      return "手繪";
    case "stamp":
      return strings.annotations.kind.stamp;
    case "textBox":
      return strings.annotations.kind.textBox;
    default:
      return strings.annotations.kind.other;
  }
}

/**
 * Formats an annotation summary as plain text or Markdown.
 */
export function formatAnnotationSummary(
  summary: AnnotationSummary,
  options: FormatSummaryOptions,
): string {
  const isMarkdown = options.format === "markdown";
  const sections: string[] = [];

  if (options.includeTitle && options.docTitle) {
    if (isMarkdown) {
      sections.push(`# ${escapeMarkdown(options.docTitle)}`);
    } else {
      sections.push(options.docTitle);
    }
  }

  if (summary.items.length === 0) {
    if (isMarkdown) {
      sections.push("*（此文件沒有註解）*");
    } else {
      sections.push("（此文件沒有註解）");
    }
  } else {
    for (const item of summary.items) {
      const page = item.pageIndex + 1;
      const label = kindName(item.kind);
      if (isMarkdown) {
        const itemContent = escapeMarkdown(item.text);
        sections.push(`## 第 ${page} 頁 [${label}]\n\n${itemContent}`);
      } else {
        sections.push(`第 ${page} 頁 [${label}]\n${item.text}`);
      }
    }
  }

  if (summary.truncated) {
    if (isMarkdown) {
      sections.push("---\n*已達大小上限，部分註解未列出*");
    } else {
      sections.push("---\n已達大小上限，部分註解未列出");
    }
  }

  return sections.join("\n\n") + "\n";
}
