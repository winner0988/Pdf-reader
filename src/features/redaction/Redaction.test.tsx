import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import {
  defaultRedactionRectAt,
  groupMarksByPage,
  rectOfPoints,
  rectOfQuad,
  type RedactionMarkItem,
} from "@/features/redaction/model";
import { RedactionConfirmDialog } from "@/features/redaction/RedactionConfirmDialog";
import { RedactionPanel } from "@/features/redaction/RedactionPanel";
import { redactionRectAt, REDACTION_HEIGHT_PT, REDACTION_WIDTH_PT } from "@/features/annotations/tools";
import { strings } from "@/i18n/zh-TW";
import type { Quad, Rect, RedactionSummary } from "@/ipc/generated/contract";

describe("redaction geometry and model helpers", () => {
  it("rectOfQuad correctly calculates bounding box from quad", () => {
    const quad: Quad = {
      ul: { x: 10, y: 20 },
      ur: { x: 50, y: 20 },
      ll: { x: 10, y: 40 },
      lr: { x: 50, y: 40 },
    };
    const rect = rectOfQuad(quad);
    expect(rect.x0).toBe(10);
    expect(rect.y0).toBe(20);
    expect(rect.x1).toBe(50);
    expect(rect.y1).toBe(40);
  });

  it("rectOfPoints handles arbitrary point order", () => {
    const rect = rectOfPoints({ x: 100, y: 200 }, { x: 50, y: 80 });
    expect(rect.x0).toBe(50);
    expect(rect.y0).toBe(80);
    expect(rect.x1).toBe(100);
    expect(rect.y1).toBe(200);
  });

  it("defaultRedactionRectAt clamps to page bounds", () => {
    const rect = defaultRedactionRectAt({ x: 450, y: 680 }, { widthPt: 500, heightPt: 700 });
    expect(rect.x0).toBeGreaterThanOrEqual(0);
    expect(rect.y0).toBeGreaterThanOrEqual(0);
    expect(rect.x1).toBeLessThanOrEqual(500);
    expect(rect.y1).toBeLessThanOrEqual(700);
    expect(rect.x1 - rect.x0).toBeCloseTo(REDACTION_WIDTH_PT);
    expect(rect.y1 - rect.y0).toBeCloseTo(REDACTION_HEIGHT_PT);
  });

  it("redactionRectAt places center or clamps", () => {
    const rect = redactionRectAt({ x: 250, y: 350 }, { widthPt: 500, heightPt: 700 });
    expect(rect.x1 - rect.x0).toBeCloseTo(REDACTION_WIDTH_PT);
    expect(rect.y1 - rect.y0).toBeCloseTo(REDACTION_HEIGHT_PT);
  });

  it("groupMarksByPage groups and sorts marks by page index", () => {
    const makeMark = (page: number, id: number, rect: Rect): RedactionMarkItem => ({
      page,
      annotation: {
        id,
        kind: "redaction",
        rect,
        color: null,
        text: null,
        picture: null,
      },
    });

    const marks: RedactionMarkItem[] = [
      makeMark(2, 1, { x0: 0, y0: 0, x1: 10, y1: 10 }),
      makeMark(0, 2, { x0: 0, y0: 0, x1: 10, y1: 10 }),
      makeMark(2, 3, { x0: 10, y0: 10, x1: 20, y1: 20 }),
    ];
    const grouped = groupMarksByPage(marks);
    expect(grouped).toHaveLength(2);
    expect(grouped[0]!.page).toBe(0);
    expect(grouped[0]!.marks).toHaveLength(1);
    expect(grouped[1]!.page).toBe(2);
    expect(grouped[1]!.marks).toHaveLength(2);
  });
});

describe("RedactionConfirmDialog", () => {
  const summaryWithReaching: RedactionSummary = {
    marks: 4,
    pages: 2,
    annotations: 2,
    fields: 1,
  };

  const summaryZeroReaching: RedactionSummary = {
    marks: 1,
    pages: 1,
    annotations: 0,
    fields: 0,
  };

  it("renders 4 core disclosure statements and calls onConfirm / onCancel", async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();

    const { rerender } = render(
      <RedactionConfirmDialog
        open={true}
        summary={summaryWithReaching}
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );

    // 1. Reaching deletions statement with non-zero counts
    expect(
      screen.getByText(
        strings.redaction.dialog.reachingDeletions(2, 1),
      ),
    ).toBeInTheDocument();

    // 2. 5 scope caveats
    expect(screen.getByText(strings.redaction.dialog.scopeCaveatsTitle)).toBeInTheDocument();
    for (const caveat of strings.redaction.dialog.scopeCaveats) {
      expect(screen.getByText(caveat)).toBeInTheDocument();
    }

    // 3. Undo caveat
    expect(screen.getByText(strings.redaction.dialog.undoCaveat)).toBeInTheDocument();

    // 4. Disk byte wipe caveat
    expect(screen.getByText(strings.redaction.dialog.diskCaveat)).toBeInTheDocument();

    // Confirm button triggers onConfirm
    await userEvent.click(
      screen.getByRole("button", { name: strings.redaction.dialog.confirm }),
    );
    expect(onConfirm).toHaveBeenCalledTimes(1);

    // Cancel button triggers onCancel
    rerender(
      <RedactionConfirmDialog
        open={true}
        summary={summaryZeroReaching}
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );
    expect(screen.getByText(strings.redaction.dialog.reachingNone)).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: strings.redaction.dialog.cancel }),
    );
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});

describe("RedactionPanel", () => {
  it("renders empty state when there are no marks", () => {
    render(
      <RedactionPanel
        marks={[]}
        onDeleteMark={vi.fn()}
        onDeleteAll={vi.fn()}
        onApply={vi.fn()}
        onJumpToPage={vi.fn()}
      />,
    );

    expect(screen.getByText(strings.redaction.noMarks)).toBeInTheDocument();
    const applyButton = screen.getByRole("button", { name: strings.redaction.apply });
    expect(applyButton).toBeDisabled();
    const clearButton = screen.getByRole("button", { name: strings.redaction.deleteAll });
    expect(clearButton).toBeDisabled();
  });

  it("renders disabled banner when document is signed or author restricted", () => {
    render(
      <RedactionPanel
        marks={[]}
        disabledReason={strings.redaction.signedDocument}
        allowed={false}
        onDeleteMark={vi.fn()}
        onDeleteAll={vi.fn()}
        onApply={vi.fn()}
        onJumpToPage={vi.fn()}
      />,
    );

    expect(screen.getByText(strings.redaction.signedDocument)).toBeInTheDocument();
  });

  it("renders marks grouped by page, jumps to page, deletes single mark, and applies", async () => {
    const onJumpToPage = vi.fn();
    const onDeleteMark = vi.fn();
    const onDeleteAll = vi.fn();
    const onApply = vi.fn();

    const marks: RedactionMarkItem[] = [
      {
        page: 0,
        annotation: {
          id: 1,
          kind: "redaction",
          rect: { x0: 10, y0: 10, x1: 100, y1: 100 },
          color: null,
          text: null,
          picture: null,
        },
      },
      {
        page: 1,
        annotation: {
          id: 2,
          kind: "redaction",
          rect: { x0: 20, y0: 20, x1: 200, y1: 200 },
          color: null,
          text: null,
          picture: null,
        },
      },
    ];

    render(
      <RedactionPanel
        marks={marks}
        onDeleteMark={onDeleteMark}
        onDeleteAll={onDeleteAll}
        onApply={onApply}
        onJumpToPage={onJumpToPage}
      />,
    );

    // Summary count
    expect(screen.getByText(strings.redaction.marksSummary(2, 2))).toBeInTheDocument();

    // Clicking page row jumps to page
    const page1Button = screen.getByRole("button", {
      name: strings.redaction.pageMarks(1, 1),
    });
    await userEvent.click(page1Button);
    expect(onJumpToPage).toHaveBeenCalledWith(1);

    // Delete single mark
    const deleteButton = screen.getByRole("button", {
      name: "刪除第 1 頁標記 1",
    });
    await userEvent.click(deleteButton);
    expect(onDeleteMark).toHaveBeenCalledWith(0, 1);

    // Delete all
    const deleteAllButton = screen.getByRole("button", { name: strings.redaction.deleteAll });
    await userEvent.click(deleteAllButton);
    expect(onDeleteAll).toHaveBeenCalledTimes(1);

    // Apply button
    const applyButton = screen.getByRole("button", { name: strings.redaction.apply });
    expect(applyButton).toBeEnabled();
    await userEvent.click(applyButton);
    expect(onApply).toHaveBeenCalledTimes(1);
  });

  it("calls onOpenFindPersonalData when find personal data button is clicked", async () => {
    const onOpenFindPersonalData = vi.fn();
    render(
      <RedactionPanel
        marks={[]}
        onDeleteMark={vi.fn()}
        onDeleteAll={vi.fn()}
        onApply={vi.fn()}
        onJumpToPage={vi.fn()}
        onOpenFindPersonalData={onOpenFindPersonalData}
      />,
    );

    const findButton = screen.getByRole("button", { name: strings.redaction.findPersonalData });
    expect(findButton).toBeEnabled();
    await userEvent.click(findButton);
    expect(onOpenFindPersonalData).toHaveBeenCalledTimes(1);
  });
});
