// Which pages to print (MVP-17, docs/architecture/printing.md) or export (B2-04): all of them,
// the current one, or a list such as "1-3, 5". Pages go in document order, each once.

/** Pages are rendered into memory before printing: this many at most in one go. */
export const MAX_PRINT_PAGES = 300;

export type PrintRange = { kind: "all" } | { kind: "current" } | { kind: "pages"; text: string };

/** The text that names `pages` (0-based, in document order, each once) as a list: "2-4, 6". */
export function formatPageList(pages: readonly number[]): string {
  const parts: string[] = [];
  for (let at = 0; at < pages.length; ) {
    let end = at;
    while (end + 1 < pages.length && pages[end + 1] === pages[end]! + 1) end++;
    parts.push(end > at ? `${pages[at]! + 1}-${pages[end]! + 1}` : `${pages[at]! + 1}`);
    at = end + 1;
  }
  return parts.join(", ");
}

/** 0-based page indexes in document order, or why the range cannot be printed. */
export type RangeResult = { pages: number[] } | { error: "invalid" | "tooMany" };

const PART = /^(\d+)(?:-(\d+))?$/;

/** The pages of a list such as "1-3, 5" (1-based), or null if it is not one. Stops counting a
 * little past `max`, so a huge range costs nothing. */
export function parsePageList(text: string, pageCount: number, max = MAX_PRINT_PAGES): number[] | null {
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
    for (let page = first; page <= last && pages.size <= max; page++) pages.add(page - 1);
  }
  return [...pages].sort((a, b) => a - b);
}

/** The pages of `range`, at most `max` of them. */
export function pagesInRange(range: PrintRange, pageCount: number, currentPage: number, max: number): RangeResult {
  let pages: number[] | null;
  if (range.kind === "current") pages = [Math.min(Math.max(currentPage, 1), pageCount) - 1];
  else if (range.kind === "all") pages = Array.from({ length: Math.min(pageCount, max + 1) }, (_, i) => i);
  else pages = parsePageList(range.text, pageCount, max);
  if (pages === null || pages.length === 0) return { error: "invalid" };
  if (pages.length > max) return { error: "tooMany" };
  return { pages };
}

export function pagesToPrint(range: PrintRange, pageCount: number, currentPage: number): RangeResult {
  return pagesInRange(range, pageCount, currentPage, MAX_PRINT_PAGES);
}
