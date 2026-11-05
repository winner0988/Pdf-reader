import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type {
  AnnotationId,
  AnnotationSummary,
  AnnotationSummaryItem,
  DocumentId,
} from "@/ipc/generated/contract";
import { AnnotationsListPanel } from "./AnnotationsListPanel";
import type { AnnotationsListApi } from "./listApi";

const dummyRect = { x0: 10, y0: 10, x1: 100, y1: 50 };

const sampleItems: AnnotationSummaryItem[] = [
  {
    id: 101 as AnnotationId,
    pageIndex: 0,
    kind: "highlight",
    rect: dummyRect,
    text: "Highlighted text on page 1",
  },
  {
    id: 102 as AnnotationId,
    pageIndex: 0,
    kind: "note",
    rect: dummyRect,
    text: "Note content on page 1",
  },
  {
    id: 103 as AnnotationId,
    pageIndex: 1,
    kind: "textBox",
    rect: dummyRect,
    text: "Text box on page 2",
  },
  {
    id: 104 as AnnotationId,
    pageIndex: 1,
    kind: "ink",
    rect: dummyRect,
    text: "",
  },
  {
    id: 105 as AnnotationId,
    pageIndex: 2,
    kind: "stamp",
    rect: dummyRect,
    text: "",
  },
  {
    id: 106 as AnnotationId,
    pageIndex: 2,
    kind: "rectangle",
    rect: dummyRect,
    text: "",
  },
  {
    id: 107 as AnnotationId,
    pageIndex: 3,
    kind: "underline",
    rect: dummyRect,
    text: "Underlined text on page 4",
  },
  {
    id: 108 as AnnotationId,
    pageIndex: 3,
    kind: "arrow",
    rect: dummyRect,
    text: "",
  },
];

function makeApi(summary: AnnotationSummary): AnnotationsListApi {
  return {
    getAnnotationSummary: vi.fn().mockResolvedValue(summary),
  };
}

describe("AnnotationsListPanel", () => {
  it("renders empty state when there are no annotations", async () => {
    const api = makeApi({ items: [], truncated: false });
    render(<AnnotationsListPanel doc={1 as DocumentId} api={api} delayMs={0} />);

    await waitFor(() => {
      expect(screen.getByText("這份文件沒有註解")).toBeInTheDocument();
    });
  });

  it("renders annotations sorted by page with kinds and text", async () => {
    const api = makeApi({ items: sampleItems, truncated: false });
    render(<AnnotationsListPanel doc={1 as DocumentId} api={api} delayMs={0} />);

    await waitFor(() => {
      expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(8);
    });

    const items = screen.getAllByTestId("annotation-list-item");
    expect(items[0]).toHaveTextContent("螢光筆標示");
    expect(items[0]).toHaveTextContent("第 1 頁");
    expect(items[0]).toHaveTextContent("Highlighted text on page 1");

    expect(items[1]).toHaveTextContent("附註");
    expect(items[1]).toHaveTextContent("第 1 頁");
    expect(items[1]).toHaveTextContent("Note content on page 1");

    expect(items[3]).toHaveTextContent("手繪線條");
    expect(items[3]).toHaveTextContent("第 2 頁");

    expect(items[5]).toHaveTextContent("矩形");
    expect(items[5]).toHaveTextContent("第 3 頁");
  });

  it("filters annotations by kind", async () => {
    const user = userEvent.setup();
    const api = makeApi({ items: sampleItems, truncated: false });
    render(<AnnotationsListPanel doc={1 as DocumentId} api={api} delayMs={0} />);

    await waitFor(() => {
      expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(8);
    });

    const select = screen.getByTestId("annotation-filter-select");

    // Filter by textMark (highlight + underline)
    await user.selectOptions(select, "textMark");
    expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(2);

    // Filter by note
    await user.selectOptions(select, "note");
    expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(1);
    expect(screen.getByText("Note content on page 1")).toBeInTheDocument();

    // Filter by textBox
    await user.selectOptions(select, "textBox");
    expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(1);
    expect(screen.getByText("Text box on page 2")).toBeInTheDocument();

    // Filter by ink
    await user.selectOptions(select, "ink");
    expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(1);

    // Filter by stamp
    await user.selectOptions(select, "stamp");
    expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(1);

    // Filter by shape (rectangle + arrow)
    await user.selectOptions(select, "shape");
    expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(2);

    // Back to all
    await user.selectOptions(select, "all");
    expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(8);
  });

  it("shows truncated banner when summary is truncated", async () => {
    const api = makeApi({ items: sampleItems.slice(0, 2), truncated: true });
    render(<AnnotationsListPanel doc={1 as DocumentId} api={api} delayMs={0} />);

    await waitFor(() => {
      expect(screen.getByTestId("annotation-truncated-banner")).toBeInTheDocument();
      expect(screen.getByText("註解過多，只列出前面的一部分")).toBeInTheDocument();
    });
  });

  it("places full text in accessible label and title", async () => {
    const longText = "A".repeat(200);
    const api = makeApi({
      items: [
        {
          id: 1 as AnnotationId,
          pageIndex: 0,
          kind: "highlight",
          rect: dummyRect,
          text: longText,
        },
      ],
      truncated: false,
    });
    render(<AnnotationsListPanel doc={1 as DocumentId} api={api} delayMs={0} />);

    await waitFor(() => {
      const item = screen.getByTestId("annotation-list-item");
      expect(item).toHaveAttribute("aria-label", `第 1 頁 · 螢光筆標示：${longText}`);
      const textElem = item.querySelector("p");
      expect(textElem).toHaveAttribute("title", longText);
    });
  });

  it("prevents copy and has select-none class", async () => {
    const api = makeApi({ items: sampleItems, truncated: false });
    render(<AnnotationsListPanel doc={1 as DocumentId} api={api} delayMs={0} />);

    await waitFor(() => {
      expect(screen.getByTestId("annotations-list-panel")).toBeInTheDocument();
    });

    const panel = screen.getByTestId("annotations-list-panel");
    expect(panel).toHaveClass("select-none");

    const copyEvent = fireEvent.copy(panel);
    expect(copyEvent).toBe(false); // defaultPrevented is true
  });

  it("supports keyboard navigation with Arrow keys, Home, End, and Enter", async () => {
    const onJumpToAnnotation = vi.fn();
    const api = makeApi({ items: sampleItems, truncated: false });
    render(
      <AnnotationsListPanel
        doc={1 as DocumentId}
        api={api}
        delayMs={0}
        onJumpToAnnotation={onJumpToAnnotation}
      />,
    );

    await waitFor(() => {
      expect(screen.getAllByTestId("annotation-list-item")).toHaveLength(8);
    });

    const listbox = screen.getByRole("listbox");

    // Press ArrowDown to move to second item
    fireEvent.keyDown(listbox, { key: "ArrowDown" });
    const items = screen.getAllByTestId("annotation-list-item");
    expect(items[1]).toHaveAttribute("aria-selected", "true");

    // Press Enter to activate second item
    fireEvent.keyDown(listbox, { key: "Enter" });
    expect(onJumpToAnnotation).toHaveBeenCalledWith(0, 102);

    // Press End to move to last item
    fireEvent.keyDown(listbox, { key: "End" });
    expect(items[7]).toHaveAttribute("aria-selected", "true");

    // Press Home to move to first item
    fireEvent.keyDown(listbox, { key: "Home" });
    expect(items[0]).toHaveAttribute("aria-selected", "true");

    // Click on item
    fireEvent.click(items[2]!);
    expect(onJumpToAnnotation).toHaveBeenCalledWith(1, 103);
  });

  it("discards outdated summary responses when doc changes", async () => {
    let resolveFirst: (s: AnnotationSummary) => void = () => {};
    let resolveSecond: (s: AnnotationSummary) => void = () => {};

    const api: AnnotationsListApi = {
      getAnnotationSummary: vi.fn().mockImplementation((doc: DocumentId) => {
        if (doc === 1) {
          return new Promise((res) => {
            resolveFirst = res;
          });
        }
        return new Promise((res) => {
          resolveSecond = res;
        });
      }),
    };

    const { rerender } = render(
      <AnnotationsListPanel doc={1 as DocumentId} api={api} delayMs={0} />,
    );

    // Doc changes to 2 before first resolves
    rerender(<AnnotationsListPanel doc={2 as DocumentId} api={api} delayMs={0} />);

    // Now first resolves with old items
    resolveFirst({
      items: [sampleItems[0]!],
      truncated: false,
    });

    // Old response should NOT appear
    await new Promise((r) => setTimeout(r, 10));
    expect(screen.queryByText("Highlighted text on page 1")).not.toBeInTheDocument();

    // Second resolves with new item
    resolveSecond({
      items: [sampleItems[2]!],
      truncated: false,
    });

    await waitFor(() => {
      expect(screen.getByText("Text box on page 2")).toBeInTheDocument();
    });
  });
});
