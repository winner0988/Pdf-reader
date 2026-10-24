import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PageMarksDialog } from "./PageMarksDialog";
import { strings } from "@/i18n/zh-TW";

const t = strings.pageMarks;

describe("PageMarksDialog", () => {
  it("renders preview and enables confirm button by default", async () => {
    const onPageMarksCopy = vi.fn().mockResolvedValue(true);
    const onFinished = vi.fn();
    const onOpenChange = vi.fn();

    render(
      <PageMarksDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={1}
        totalPages={10}
        onPageMarksCopy={onPageMarksCopy}
        onFinished={onFinished}
      />,
    );

    expect(screen.getByRole("heading", { name: t.title })).toBeInTheDocument();
    expect(screen.getByText(t.firstPagePreview + "（第 1 頁）")).toBeInTheDocument();
    expect(screen.getByText(t.lastPagePreview + "（第 10 頁）")).toBeInTheDocument();

    const submitBtn = screen.getByRole("button", { name: t.start });
    expect(submitBtn).toBeEnabled();
  });

  it("disables confirm button when a slot collision occurs", async () => {
    const user = userEvent.setup();
    render(
      <PageMarksDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        totalPages={10}
        onPageMarksCopy={vi.fn()}
        onFinished={vi.fn()}
      />,
    );

    const submitBtn = screen.getByRole("button", { name: t.start });
    expect(submitBtn).toBeEnabled();

    // Enable Bates number
    const batesCheckbox = screen.getByRole("checkbox", { name: t.batesTitle });
    await user.click(batesCheckbox);

    // By default Bates is at BottomRight while page number is at BottomCenter -> no collision
    expect(submitBtn).toBeEnabled();

    // Change Bates position to BottomCenter (same as page number)
    const selects = screen.getAllByRole("combobox");
    const batesPosSelect = selects.find(
      (sel) => (sel as HTMLSelectElement).value === "bottomRight",
    );
    expect(batesPosSelect).toBeDefined();
    await user.selectOptions(batesPosSelect!, "bottomCenter");

    // Collision should be detected and confirm button disabled!
    expect(screen.getByText(new RegExp(t.conflictError))).toBeInTheDocument();
    expect(submitBtn).toBeDisabled();
  });

  it("disables confirm button when Bates number overflows available digits", async () => {
    const user = userEvent.setup();
    render(
      <PageMarksDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        totalPages={10}
        onPageMarksCopy={vi.fn()}
        onFinished={vi.fn()}
      />,
    );

    // Enable Bates number
    const batesCheckbox = screen.getByRole("checkbox", { name: t.batesTitle });
    await user.click(batesCheckbox);

    // Change digits to 1 (10 pages with 1 digit starting at 1 -> overflow!)
    const digitsInput = screen.getByDisplayValue("6");
    fireEvent.change(digitsInput, { target: { value: "1" } });

    expect(screen.getByText(t.batesOverflow)).toBeInTheDocument();
    const submitBtn = screen.getByRole("button", { name: t.start });
    expect(submitBtn).toBeDisabled();
  });

  it("submits valid page marks and closes dialog on success", async () => {
    const user = userEvent.setup();
    const onPageMarksCopy = vi.fn().mockResolvedValue(true);
    const onFinished = vi.fn();
    const onOpenChange = vi.fn();

    render(
      <PageMarksDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={42}
        totalPages={5}
        onPageMarksCopy={onPageMarksCopy}
        onFinished={onFinished}
      />,
    );

    const submitBtn = screen.getByRole("button", { name: t.start });
    await user.click(submitBtn);

    expect(onPageMarksCopy).toHaveBeenCalledWith(
      expect.objectContaining({
        doc: 42,
        marks: expect.objectContaining({
          pages: { kind: "all" },
          pageNumber: expect.objectContaining({
            format: { kind: "pageOfTotal" },
            start: 1,
            position: "bottomCenter",
          }),
        }),
      }),
    );

    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onFinished).toHaveBeenCalledWith(t.done);
  });
});
