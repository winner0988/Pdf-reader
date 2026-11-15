import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { PrintPages } from "@/features/print/PrintPages";
import type { PrintPage } from "@/features/print/render";

function mockPages(count: number): PrintPage[] {
  return Array.from({ length: count }, (_, i) => ({
    pageIndex: i,
    url: `blob:page-${i}`,
    size: { widthPt: 600, heightPt: 800 },
  }));
}

describe("PrintPages", () => {
  it("renders 1-up layout by default", () => {
    const pages = mockPages(2);
    render(<PrintPages pages={pages} onDone={vi.fn()} />);

    const container = document.querySelector("[data-print-pages]");
    expect(container).toBeInTheDocument();

    const sheets = container?.querySelectorAll("[data-print-sheet]");
    expect(sheets).toHaveLength(2);

    const images = container?.querySelectorAll("img");
    expect(images).toHaveLength(2);
    expect(images?.[0]?.getAttribute("data-page")).toBe("1");
    expect(images?.[1]?.getAttribute("data-page")).toBe("2");
  });

  it("renders 4-up grid layout with cells and borders", () => {
    const pages = mockPages(4);
    render(
      <PrintPages
        pages={pages}
        layout="4"
        order="vertical"
        border={true}
        onDone={vi.fn()}
      />,
    );

    const container = document.querySelector("[data-print-pages]");
    const sheets = container?.querySelectorAll("[data-print-sheet]");
    expect(sheets).toHaveLength(1);

    const cells = sheets?.[0]?.querySelectorAll("[data-print-cell]");
    expect(cells).toHaveLength(4);

    // Border attribute should be present
    cells?.forEach((cell) => {
      expect(cell.getAttribute("data-border")).toBe("true");
    });

    // In 4-up vertical order: cell 0=page 1, cell 1=page 3, cell 2=page 2, cell 3=page 4
    const pageNumbers = Array.from(cells ?? []).map(
      (cell) => cell.querySelector("img")?.getAttribute("data-page"),
    );
    expect(pageNumbers).toEqual(["1", "3", "2", "4"]);
  });

  it("renders booklet layout with 4 faces for 8 pages and includes landscape style", () => {
    const pages = mockPages(8);
    render(
      <PrintPages
        pages={pages}
        layout="booklet"
        onDone={vi.fn()}
      />,
    );

    const container = document.querySelector("[data-print-pages]");
    const style = container?.querySelector("style");
    expect(style?.textContent).toContain("@page { size: landscape; }");

    const sheets = container?.querySelectorAll("[data-print-sheet]");
    expect(sheets).toHaveLength(4);

    const sheetPages = Array.from(sheets ?? []).map((sheet) => {
      const cells = sheet.querySelectorAll("[data-print-cell]");
      expect(cells).toHaveLength(2);
      return Array.from(cells).map((cell) => cell.querySelector("img")?.getAttribute("data-page") ?? null);
    });

    // Exactly matches prompt:
    // Face 0: [8, 1]
    // Face 1: [2, 7]
    // Face 2: [6, 3]
    // Face 3: [4, 5]
    expect(sheetPages).toEqual([
      ["8", "1"],
      ["2", "7"],
      ["6", "3"],
      ["4", "5"],
    ]);
  });
});
