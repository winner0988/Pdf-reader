import type { AnnotationSummaryKind } from "@/ipc/generated/contract";

export type AnnotationFilterKind =
  | "all"
  | "textMark"
  | "note"
  | "textBox"
  | "ink"
  | "stamp"
  | "shape";

export function matchesFilter(kind: AnnotationSummaryKind, filter: AnnotationFilterKind): boolean {
  switch (filter) {
    case "all":
      return true;
    case "textMark":
      return (
        kind === "highlight" ||
        kind === "underline" ||
        kind === "strikeOut" ||
        kind === "squiggly"
      );
    case "note":
      return kind === "note";
    case "textBox":
      return kind === "textBox";
    case "ink":
      return kind === "ink";
    case "stamp":
      return kind === "stamp";
    case "shape":
      return (
        kind === "rectangle" ||
        kind === "ellipse" ||
        kind === "line" ||
        kind === "arrow"
      );
    default:
      return false;
  }
}
