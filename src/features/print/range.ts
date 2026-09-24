// Which pages to print (MVP-17, docs/architecture/printing.md): all of them, the current one, or
// a list such as "1-3, 5". Pages are printed in document order, each once.

/** Pages are rendered into memory before printing: this many at most in one go. */
export const MAX_PRINT_PAGES = 300;

export type PrintRange = { kind: "all" } | { kind: "current" } | { kind: "pages"; text: string };

/** 0-based page indexes in document order, or why the range cannot be printed. */
export type RangeResult = { pages: number[] } | { error: "invalid" | "tooMany" };

const PART = /^(\d+)(?:-(\d+))?$/;

/** The pages of a list such as "1-3, 5" (1-based), or null if it is not one. */
export function parsePageList(text: string, pageCount: number): number[] | null {
  // Spaces may surround a dash; commas, the ideographic comma and spaces separate parts.
  const parts = text
    .trim()
    .replace(/\s*[-–~～]\s*/g, "-")
    .split(/[\s,，、]+/)
    .filter((part) => part !== "");
  if (parts.length === 0) return null;
  const pages = new Set<number>();
  for (const part of parts) {
    const match = PART.exec(part);
    if (!match) return null;
    const first = Number(match[1]);
    const last = match[2] === undefined ? first : Number(match[2]);
    if (first < 1 || last < first || last > pageCount) return null;
    for (let page = first; page <= last && pages.size <= MAX_PRINT_PAGES; page++) pages.add(page - 1);
  }
  return [...pages].sort((a, b) => a - b);
}

export function pagesToPrint(range: PrintRange, pageCount: number, currentPage: number): RangeResult {
  let pages: number[] | null;
  if (range.kind === "current") pages = [Math.min(Math.max(currentPage, 1), pageCount) - 1];
  else if (range.kind === "all") pages = Array.from({ length: Math.min(pageCount, MAX_PRINT_PAGES + 1) }, (_, i) => i);
  else pages = parsePageList(range.text, pageCount);
  if (pages === null || pages.length === 0) return { error: "invalid" };
  if (pages.length > MAX_PRINT_PAGES) return { error: "tooMany" };
  return { pages };
}
