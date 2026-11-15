// Print layout calculation (B5-15, Issue #452, docs/architecture/printing.md):
// N-up (1, 2, 4, 6, 9 pages per sheet) and booklet printing.
// Pure function decoupled from the React view, to ease testing.

export type PrintLayoutKind = "1" | "2" | "4" | "6" | "9" | "booklet";

/**
 * Ordering of pages on an N-up sheet:
 * - "horizontal": across rows first (left to right, then top to bottom) - 先橫後直
 * - "vertical": down columns first (top to bottom, then left to right) - 先直後橫
 */
export type PrintPageOrder = "horizontal" | "vertical";

export type PrintSheet<T> = {
  /** 0-based index of this sheet / printed face */
  sheetIndex: number;
  rows: number;
  cols: number;
  /**
   * Cells in row-major order: length === rows * cols.
   * Empty / padding cells are null.
   */
  cells: (T | null)[];
  orientation: "portrait" | "landscape";
};

export const LAYOUT_GRID: Record<
  PrintLayoutKind,
  { rows: number; cols: number; orientation: "portrait" | "landscape" }
> = {
  "1": { rows: 1, cols: 1, orientation: "portrait" },
  "2": { rows: 1, cols: 2, orientation: "landscape" },
  "4": { rows: 2, cols: 2, orientation: "portrait" },
  "6": { rows: 2, cols: 3, orientation: "landscape" },
  "9": { rows: 3, cols: 3, orientation: "portrait" },
  booklet: { rows: 1, cols: 2, orientation: "landscape" },
};

/**
 * Calculates the sheets (printed faces) and cell placements for the given pages.
 *
 * For N-up:
 * - 1 page: 1x1
 * - 2 pages: 1x2 (landscape recommended)
 * - 4 pages: 2x2
 * - 6 pages: 2x3 (landscape recommended)
 * - 9 pages: 3x3
 * Any unfilled slots on the final sheet are filled with null (blank).
 *
 * For booklet:
 * - Total pages padded to the next multiple of 4 (n = ceil(M / 4) * 4).
 * - Each physical sheet has 2 faces (front and back), 1x2 side-by-side.
 * - For sheet k (0-based, k = 0 .. n/4 - 1):
 *     Front face: [n - 2*k, 2*k + 1] (1-based position in selection)
 *     Back face:  [2*k + 2, n - 2*k - 1]
 * - Blank padding positions (> M) are null.
 */
export function calculatePrintLayout<T>(
  pages: readonly T[],
  layout: PrintLayoutKind = "1",
  order: PrintPageOrder = "horizontal",
): PrintSheet<T>[] {
  if (pages.length === 0) return [];

  if (layout === "booklet") {
    const m = pages.length;
    const n = Math.ceil(m / 4) * 4;
    const sheetCount = n / 4;
    const getPage = (pos: number): T | null => (pos >= 1 && pos <= m ? pages[pos - 1]! : null);

    const sheets: PrintSheet<T>[] = [];
    for (let k = 0; k < sheetCount; k++) {
      // Front face
      sheets.push({
        sheetIndex: sheets.length,
        rows: 1,
        cols: 2,
        cells: [getPage(n - 2 * k), getPage(2 * k + 1)],
        orientation: "landscape",
      });
      // Back face
      sheets.push({
        sheetIndex: sheets.length,
        rows: 1,
        cols: 2,
        cells: [getPage(2 * k + 2), getPage(n - 2 * k - 1)],
        orientation: "landscape",
      });
    }
    return sheets;
  }

  const { rows, cols, orientation } = LAYOUT_GRID[layout];
  const capacity = rows * cols;
  const sheetCount = Math.ceil(pages.length / capacity);
  const sheets: PrintSheet<T>[] = [];

  for (let s = 0; s < sheetCount; s++) {
    const slice = pages.slice(s * capacity, (s + 1) * capacity);
    const cells = new Array<T | null>(capacity).fill(null);

    for (let i = 0; i < slice.length; i++) {
      let cellIndex: number;
      if (order === "vertical") {
        const r = i % rows;
        const c = Math.floor(i / rows);
        cellIndex = r * cols + c;
      } else {
        cellIndex = i;
      }
      cells[cellIndex] = slice[i]!;
    }

    sheets.push({
      sheetIndex: s,
      rows,
      cols,
      cells,
      orientation,
    });
  }

  return sheets;
}
