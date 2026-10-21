import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ExportApi } from "@/features/export/api";
import { strings } from "@/i18n/zh-TW";
import type { AnnotationSummary, DocumentId } from "@/ipc/generated/contract";
import { AnnotationSummaryDialog } from "./AnnotationSummaryDialog";

const t = strings.exportAnnotations;

describe("AnnotationSummaryDialog", () => {
  const mockDoc = 1 as DocumentId;
  const mockDocTitle = "測試文件.pdf";
  const mockSummary: AnnotationSummary = {
    items: [
      {
        pageIndex: 0,
        kind: "highlight",
        rect: { x0: 10, y0: 10, x1: 50, y1: 50 },
        text: "重點文字",
      },
      {
        pageIndex: 1,
        kind: "note",
        rect: { x0: 20, y0: 20, x1: 40, y1: 40 },
        text: "附註說明",
      },
    ],
    truncated: false,
  };

  function createMockApi(overrides?: Partial<ExportApi>): ExportApi {
    return {
      exportPages: vi.fn(),
      cancel: vi.fn(),
      getAnnotationSummary: vi.fn().mockResolvedValue(mockSummary),
      exportAnnotationSummary: vi.fn().mockResolvedValue(true),
      ...overrides,
    };
  }

  it("renders format options and title checkbox", () => {
    const api = createMockApi();
    render(
      <AnnotationSummaryDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={mockDoc}
        docTitle={mockDocTitle}
        api={api}
        onFinished={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: t.title })).toBeInTheDocument();
    expect(screen.getByText(t.description)).toBeInTheDocument();

    const textRadio = screen.getByRole("radio", { name: t.text });
    const mdRadio = screen.getByRole("radio", { name: t.markdown });
    const titleCheckbox = screen.getByRole("checkbox", { name: t.includeTitle });

    expect(textRadio).toBeChecked();
    expect(mdRadio).not.toBeChecked();
    expect(titleCheckbox).toBeChecked();
  });

  it("exports as plain text with title included by default", async () => {
    const user = userEvent.setup();
    const api = createMockApi();
    const onOpenChange = vi.fn();
    const onFinished = vi.fn();

    render(
      <AnnotationSummaryDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={mockDoc}
        docTitle={mockDocTitle}
        api={api}
        onFinished={onFinished}
      />,
    );

    const submitBtn = screen.getByRole("button", { name: t.start });
    await user.click(submitBtn);

    await waitFor(() => {
      expect(api.getAnnotationSummary).toHaveBeenCalledWith(mockDoc);
      expect(api.exportAnnotationSummary).toHaveBeenCalledWith(
        expect.objectContaining({
          doc: mockDoc,
          format: "text",
          content: expect.stringContaining("測試文件.pdf"),
        }),
      );
      expect(onOpenChange).toHaveBeenCalledWith(false);
      expect(onFinished).toHaveBeenCalledWith(t.done);
    });
  });

  it("exports as Markdown and excludes title when unchecked", async () => {
    const user = userEvent.setup();
    const api = createMockApi();
    const onOpenChange = vi.fn();
    const onFinished = vi.fn();

    render(
      <AnnotationSummaryDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={mockDoc}
        docTitle={mockDocTitle}
        api={api}
        onFinished={onFinished}
      />,
    );

    const mdRadio = screen.getByRole("radio", { name: t.markdown });
    await user.click(mdRadio);

    const titleCheckbox = screen.getByRole("checkbox", { name: t.includeTitle });
    await user.click(titleCheckbox);

    const submitBtn = screen.getByRole("button", { name: t.start });
    await user.click(submitBtn);

    await waitFor(() => {
      expect(api.exportAnnotationSummary).toHaveBeenCalledWith(
        expect.objectContaining({
          doc: mockDoc,
          format: "markdown",
          content: expect.not.stringContaining("# 測試文件.pdf"),
        }),
      );
      expect(onOpenChange).toHaveBeenCalledWith(false);
      expect(onFinished).toHaveBeenCalledWith(t.done);
    });
  });

  it("displays error message when export fails", async () => {
    const user = userEvent.setup();
    const api = createMockApi({
      exportAnnotationSummary: vi.fn().mockRejectedValue(new Error("fail")),
    });
    const onOpenChange = vi.fn();
    const onFinished = vi.fn();

    render(
      <AnnotationSummaryDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={mockDoc}
        docTitle={mockDocTitle}
        api={api}
        onFinished={onFinished}
      />,
    );

    const submitBtn = screen.getByRole("button", { name: t.start });
    await user.click(submitBtn);

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(t.failed);
      expect(onOpenChange).not.toHaveBeenCalled();
      expect(onFinished).not.toHaveBeenCalled();
    });
  });
});
