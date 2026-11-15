import { describe, expect, it } from "vitest";

import { calculatePrintLayout } from "@/features/print/layout";
import { MAX_PRINT_PAGES } from "@/features/print/range";

describe("print layout calculations", () => {
  it("calculates grid dimensions and cell count for each layout", () => {
    // 1-up: 1x1
    const layout1 = calculatePrintLayout([1], "1");
    expect(layout1).toHaveLength(1);
    expect(layout1[0]).toMatchObject({ rows: 1, cols: 1, orientation: "portrait", cells: [1] });

    // 2-up: 1x2 landscape
    const layout2 = calculatePrintLayout([1, 2], "2");
    expect(layout2).toHaveLength(1);
    expect(layout2[0]).toMatchObject({ rows: 1, cols: 2, orientation: "landscape", cells: [1, 2] });

    // 4-up: 2x2 portrait
    const layout4 = calculatePrintLayout([1, 2, 3, 4], "4");
    expect(layout4).toHaveLength(1);
    expect(layout4[0]).toMatchObject({ rows: 2, cols: 2, orientation: "portrait", cells: [1, 2, 3, 4] });

    // 6-up: 2x3 landscape
    const layout6 = calculatePrintLayout([1, 2, 3, 4, 5, 6], "6");
    expect(layout6).toHaveLength(1);
    expect(layout6[0]).toMatchObject({ rows: 2, cols: 3, orientation: "landscape", cells: [1, 2, 3, 4, 5, 6] });

    // 9-up: 3x3 portrait
    const layout9 = calculatePrintLayout([1, 2, 3, 4, 5, 6, 7, 8, 9], "9");
    expect(layout9).toHaveLength(1);
    expect(layout9[0]).toMatchObject({
      rows: 3,
      cols: 3,
      orientation: "portrait",
      cells: [1, 2, 3, 4, 5, 6, 7, 8, 9],
    });
  });

  it("leaves empty slots on the last sheet as null", () => {
    // 4-up with 3 pages -> 1 sheet with 1 null cell
    const sheets4 = calculatePrintLayout([1, 2, 3], "4");
    expect(sheets4).toHaveLength(1);
    expect(sheets4[0]!.cells).toEqual([1, 2, 3, null]);

    // 6-up with 7 pages -> 2 sheets, second sheet has 1 page and 5 null cells
    const sheets6 = calculatePrintLayout([1, 2, 3, 4, 5, 6, 7], "6");
    expect(sheets6).toHaveLength(2);
    expect(sheets6[0]!.cells).toEqual([1, 2, 3, 4, 5, 6]);
    expect(sheets6[1]!.cells).toEqual([7, null, null, null, null, null]);
  });

  it("supports both horizontal (across) and vertical (down) page orders", () => {
    const pages = [1, 2, 3, 4, 5, 6];

    // Horizontal (row-first):
    // [1, 2, 3]
    // [4, 5, 6]
    const horiz = calculatePrintLayout(pages, "6", "horizontal");
    expect(horiz[0]!.cells).toEqual([1, 2, 3, 4, 5, 6]);

    // Vertical (col-first):
    // [1, 3, 5]
    // [2, 4, 6]
    // In row-major cells array: [1, 3, 5, 2, 4, 6]
    const vert = calculatePrintLayout(pages, "6", "vertical");
    expect(vert[0]!.cells).toEqual([1, 3, 5, 2, 4, 6]);

    // 4-up (2x2):
    // Horizontal:
    // [1, 2]
    // [3, 4]
    expect(calculatePrintLayout([1, 2, 3, 4], "4", "horizontal")[0]!.cells).toEqual([1, 2, 3, 4]);

    // Vertical:
    // [1, 3]
    // [2, 4]
    expect(calculatePrintLayout([1, 2, 3, 4], "4", "vertical")[0]!.cells).toEqual([1, 3, 2, 4]);

    // Vertical with incomplete sheet: 3 pages in 4-up (2x2)
    // Col 0: 1 then 2
    // Col 1: 3 then null
    // Row 0: 1, 3
    // Row 1: 2, null
    expect(calculatePrintLayout([1, 2, 3], "4", "vertical")[0]!.cells).toEqual([1, 3, 2, null]);
  });

  it("calculates exact booklet page sequences for 1, 2, 3, 4, 5, 8, 9, 16 pages", () => {
    const toCells = (pages: number[]) => calculatePrintLayout(pages, "booklet").map((s) => s.cells);

    // 1 page (padded to 4 -> 1 sheet, 2 faces)
    expect(toCells([1])).toEqual([
      [null, 1],
      [null, null],
    ]);

    // 2 pages (padded to 4 -> 1 sheet, 2 faces)
    expect(toCells([1, 2])).toEqual([
      [null, 1],
      [2, null],
    ]);

    // 3 pages (padded to 4 -> 1 sheet, 2 faces)
    expect(toCells([1, 2, 3])).toEqual([
      [null, 1],
      [2, 3],
    ]);

    // 4 pages (4 pages -> 1 sheet, 2 faces)
    expect(toCells([1, 2, 3, 4])).toEqual([
      [4, 1],
      [2, 3],
    ]);

    // 5 pages (padded to 8 -> 2 sheets, 4 faces)
    expect(toCells([1, 2, 3, 4, 5])).toEqual([
      [null, 1],
      [2, null],
      [null, 3],
      [4, 5],
    ]);

    // 8 pages (8 pages -> 2 sheets, 4 faces) - exactly matches prompt example:
    // Sheet 0 front: [8, 1]; back: [2, 7]
    // Sheet 1 front: [6, 3]; back: [4, 5]
    expect(toCells([1, 2, 3, 4, 5, 6, 7, 8])).toEqual([
      [8, 1],
      [2, 7],
      [6, 3],
      [4, 5],
    ]);

    // 9 pages (padded to 12 -> 3 sheets, 6 faces)
    expect(toCells([1, 2, 3, 4, 5, 6, 7, 8, 9])).toEqual([
      [null, 1],
      [2, null],
      [null, 3],
      [4, 9],
      [8, 5],
      [6, 7],
    ]);

    // 16 pages (16 pages -> 4 sheets, 8 faces)
    expect(toCells([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])).toEqual([
      [16, 1],
      [2, 15],
      [14, 3],
      [4, 13],
      [12, 5],
      [6, 11],
      [10, 7],
      [8, 9],
    ]);
  });

  it("maps booklet positions to the selected pages when a range like '2-5, 9' is chosen", () => {
    // 5 selected pages: [2, 3, 4, 5, 9]
    // Position 1=2, 2=3, 3=4, 4=5, 5=9
    // Padded to 8 (2 sheets, 4 faces):
    // Face 0: [null, 2]
    // Face 1: [3, null]
    // Face 2: [null, 4]
    // Face 3: [5, 9]
    const chosen = [2, 3, 4, 5, 9];
    const sheets = calculatePrintLayout(chosen, "booklet");
    expect(sheets.map((s) => s.cells)).toEqual([
      [null, 2],
      [3, null],
      [null, 4],
      [5, 9],
    ]);
  });

  it("handles empty pages array", () => {
    expect(calculatePrintLayout([], "4")).toEqual([]);
    expect(calculatePrintLayout([], "booklet")).toEqual([]);
  });

  it("works seamlessly up to MAX_PRINT_PAGES limit", () => {
    const pages = Array.from({ length: MAX_PRINT_PAGES }, (_, i) => i + 1);

    // In 4-up: 300 / 4 = 75 sheets, no remainder
    const sheets4 = calculatePrintLayout(pages, "4");
    expect(sheets4).toHaveLength(75);
    expect(sheets4[74]!.cells[3]).toBe(MAX_PRINT_PAGES);

    // In booklet: 300 is a multiple of 4 (300 / 4 = 75 physical sheets = 150 faces)
    const bookletSheets = calculatePrintLayout(pages, "booklet");
    expect(bookletSheets).toHaveLength(150);
    // Front face of sheet 0: [300, 1]
    expect(bookletSheets[0]!.cells).toEqual([300, 1]);
    // Centerfold (sheet 74 back face): [150, 151]
    expect(bookletSheets[149]!.cells).toEqual([150, 151]);
  });
});
