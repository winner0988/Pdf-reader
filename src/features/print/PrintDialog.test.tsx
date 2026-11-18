import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PrintDialog } from "@/features/print/PrintDialog";
import type { PrintPage } from "@/features/print/render";
import { strings } from "@/i18n/zh-TW";

const t = strings.print;

function mockPrintPages(pages: number[]): PrintPage[] {
  return pages.map((pageIndex) => ({
    pageIndex,
    url: `blob:page-${pageIndex}`,
    size: { widthPt: 595, heightPt: 842 },
  }));
}

describe("PrintDialog", () => {
  it("renders with default layout (1 page per sheet) and allows choosing multi-page layout and options", async () => {
    const user = userEvent.setup();
    const onReady = vi.fn();
    const prepare = vi.fn((pages: number[]) => Promise.resolve(mockPrintPages(pages)));

    render(
      <PrintDialog
        open={true}
        onOpenChange={vi.fn()}
        pageCount={4}
        currentPage={1}
        prepare={prepare}
        onReady={onReady}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: t.title });
    expect(dialog).toBeInTheDocument();

    const layoutSelect = within(dialog).getByRole("combobox", { name: t.layout });
    expect(layoutSelect).toHaveValue("1");

    // Multi-page options should not be visible when layout is 1
    expect(within(dialog).queryByRole("combobox", { name: t.order })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole("checkbox", { name: t.pageBorder })).not.toBeInTheDocument();

    // Select 4 pages per sheet
    await user.selectOptions(layoutSelect, "4");
    expect(layoutSelect).toHaveValue("4");

    // Multi-page order and border controls are now visible
    const orderSelect = within(dialog).getByRole("combobox", { name: t.order });
    expect(orderSelect).toHaveValue("horizontal");
    await user.selectOptions(orderSelect, "vertical");
    expect(orderSelect).toHaveValue("vertical");

    const borderCheckbox = within(dialog).getByRole("checkbox", { name: t.pageBorder });
    expect(borderCheckbox).not.toBeChecked();
    await user.click(borderCheckbox);
    expect(borderCheckbox).toBeChecked();

    // Submit dialog
    await user.click(within(dialog).getByRole("button", { name: t.next }));

    expect(prepare).toHaveBeenCalledWith([0, 1, 2, 3], expect.any(Function), expect.any(AbortSignal));
    expect(onReady).toHaveBeenCalledWith({
      pages: mockPrintPages([0, 1, 2, 3]),
      layout: "4",
      order: "vertical",
      border: true,
    });
  });

  it("shows booklet hint when booklet layout is selected", async () => {
    const user = userEvent.setup();
    const onReady = vi.fn();
    const prepare = vi.fn((pages: number[]) => Promise.resolve(mockPrintPages(pages)));

    render(
      <PrintDialog
        open={true}
        onOpenChange={vi.fn()}
        pageCount={8}
        currentPage={1}
        prepare={prepare}
        onReady={onReady}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: t.title });
    const layoutSelect = within(dialog).getByRole("combobox", { name: t.layout });

    // Select booklet
    await user.selectOptions(layoutSelect, "booklet");
    expect(layoutSelect).toHaveValue("booklet");

    // Multi-page order and border controls should not be visible for booklet
    expect(within(dialog).queryByRole("combobox", { name: t.order })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole("checkbox", { name: t.pageBorder })).not.toBeInTheDocument();

    // Booklet hint must be visible
    expect(within(dialog).getByText(t.bookletHint)).toBeInTheDocument();

    // Submit dialog
    await user.click(within(dialog).getByRole("button", { name: t.next }));

    expect(onReady).toHaveBeenCalledWith({
      pages: mockPrintPages([0, 1, 2, 3, 4, 5, 6, 7]),
      layout: "booklet",
      order: "horizontal",
      border: false,
    });
  });

  it("disables inputs and submit button when disabled is true", () => {
    render(
      <PrintDialog
        open={true}
        onOpenChange={vi.fn()}
        pageCount={4}
        currentPage={1}
        prepare={vi.fn()}
        onReady={vi.fn()}
        disabled={true}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: t.title });
    expect(within(dialog).getByRole("button", { name: t.next })).toBeDisabled();
    expect(within(dialog).getByRole("combobox", { name: t.layout })).toBeDisabled();
  });

  it("displays low-resolution printing note when lowResolutionDpi is specified", () => {
    render(
      <PrintDialog
        open={true}
        onOpenChange={vi.fn()}
        pageCount={4}
        currentPage={1}
        prepare={vi.fn()}
        onReady={vi.fn()}
        lowResolutionDpi={150}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: t.title });
    expect(within(dialog).getByText(strings.permissions.lowResNote(150))).toBeInTheDocument();
  });

  it("restores focus to previous element on Escape", async () => {
    const user = userEvent.setup();

    function Harness() {
      const [open, setOpen] = useState(false);
      return (
        <div>
          <button data-testid="trigger" onClick={() => setOpen(true)}>
            Open
          </button>
          <PrintDialog
            open={open}
            onOpenChange={setOpen}
            pageCount={4}
            currentPage={1}
            prepare={vi.fn()}
            onReady={vi.fn()}
          />
        </div>
      );
    }

    render(<Harness />);
    const trigger = screen.getByTestId("trigger");
    trigger.focus();
    expect(trigger).toHaveFocus();

    await user.click(trigger);
    const dialog = await screen.findByRole("dialog", { name: t.title });
    expect(dialog).toBeInTheDocument();

    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(trigger).toHaveFocus());
  });
});
