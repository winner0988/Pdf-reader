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
  it("renders 1-up layout by default with portrait orientation and no style tag", () => {
    const pages = mockPages(2);
    render(<PrintPages pages={pages} onDone={vi.fn()} />);

    const container = document.querySelector("[data-print-pages]");
    expect(container).toBeInTheDocument();
    expect(document.querySelector("style")).toBeNull();

    const sheets = container?.querySelectorAll("[data-print-sheet]");
    expect(sheets).toHaveLength(2);
    sheets?.forEach((sheet) => {
      expect(sheet.getAttribute("data-orientation")).toBe("portrait");
    });

    const images = container?.querySelectorAll("img");
    expect(images).toHaveLength(2);
    expect(images?.[0]?.getAttribute("data-page")).toBe("1");
    expect(images?.[1]?.getAttribute("data-page")).toBe("2");
  });

  it("renders 4-up and 9-up layouts with portrait orientation and no style tag", () => {
    const { unmount } = render(
      <PrintPages
        pages={mockPages(4)}
        layout="4"
        order="vertical"
        border={true}
        onDone={vi.fn()}
      />,
    );

    const container = document.querySelector("[data-print-pages]");
    expect(document.querySelector("style")).toBeNull();
    const sheets4 = container?.querySelectorAll("[data-print-sheet]");
    expect(sheets4).toHaveLength(1);
    expect(sheets4?.[0]?.getAttribute("data-orientation")).toBe("portrait");

    const cells = sheets4?.[0]?.querySelectorAll("[data-print-cell]");
    expect(cells).toHaveLength(4);

    cells?.forEach((cell) => {
      expect(cell.getAttribute("data-border")).toBe("true");
    });

    const pageNumbers = Array.from(cells ?? []).map(
      (cell) => cell.querySelector("img")?.getAttribute("data-page"),
    );
    expect(pageNumbers).toEqual(["1", "3", "2", "4"]);

    unmount();

    render(
      <PrintPages
        pages={mockPages(9)}
        layout="9"
        onDone={vi.fn()}
      />,
    );
    const container9 = document.querySelector("[data-print-pages]");
    expect(document.querySelector("style")).toBeNull();
    const sheets9 = container9?.querySelectorAll("[data-print-sheet]");
    expect(sheets9).toHaveLength(1);
    expect(sheets9?.[0]?.getAttribute("data-orientation")).toBe("portrait");
  });

  it("renders 2-up and 6-up layouts with landscape orientation and no style tag", () => {
    const { unmount } = render(
      <PrintPages
        pages={mockPages(2)}
        layout="2"
        onDone={vi.fn()}
      />,
    );

    const container2 = document.querySelector("[data-print-pages]");
    expect(document.querySelector("style")).toBeNull();
    const sheets2 = container2?.querySelectorAll("[data-print-sheet]");
    expect(sheets2).toHaveLength(1);
    expect(sheets2?.[0]?.getAttribute("data-orientation")).toBe("landscape");

    unmount();

    render(
      <PrintPages
        pages={mockPages(6)}
        layout="6"
        onDone={vi.fn()}
      />,
    );
    const container6 = document.querySelector("[data-print-pages]");
    expect(document.querySelector("style")).toBeNull();
    const sheets6 = container6?.querySelectorAll("[data-print-sheet]");
    expect(sheets6).toHaveLength(1);
    expect(sheets6?.[0]?.getAttribute("data-orientation")).toBe("landscape");
  });

  it("renders booklet layout with 4 faces for 8 pages with landscape orientation and no style tag", () => {
    const pages = mockPages(8);
    render(
      <PrintPages
        pages={pages}
        layout="booklet"
        onDone={vi.fn()}
      />,
    );

    const container = document.querySelector("[data-print-pages]");
    expect(document.querySelector("style")).toBeNull();

    const sheets = container?.querySelectorAll("[data-print-sheet]");
    expect(sheets).toHaveLength(4);

    sheets?.forEach((sheet) => {
      expect(sheet.getAttribute("data-orientation")).toBe("landscape");
    });

    const sheetPages = Array.from(sheets ?? []).map((sheet) => {
      const cells = sheet.querySelectorAll("[data-print-cell]");
      expect(cells).toHaveLength(2);
      return Array.from(cells).map((cell) => cell.querySelector("img")?.getAttribute("data-page") ?? null);
    });

    expect(sheetPages).toEqual([
      ["8", "1"],
      ["2", "7"],
      ["6", "3"],
      ["4", "5"],
    ]);
  });
});
