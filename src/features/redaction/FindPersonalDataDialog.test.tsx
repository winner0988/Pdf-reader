import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { AnnotationsApi } from "@/features/annotations/source";
import { FindPersonalDataDialog } from "@/features/redaction/FindPersonalDataDialog";
import { strings } from "@/i18n/zh-TW";
import type { PersonalDataMatches } from "@/ipc/generated/contract";

const t = strings.redaction.findPersonalDataDialog;

describe("FindPersonalDataDialog", () => {
  const sampleMatches: PersonalDataMatches = {
    matches: [
      {
        category: "taiwanId",
        page: 0,
        rects: [{ x0: 10, y0: 10, x1: 100, y1: 20 }],
        preview: "A12****789",
      },
      {
        category: "mobilePhone",
        page: 1,
        rects: [{ x0: 20, y0: 30, x1: 120, y1: 40 }],
        preview: "0912****78",
      },
    ],
    hasMore: false,
  };

  it("renders category checkboxes, allows entering keyword, and performs search", async () => {
    const findPersonalData = vi.fn().mockResolvedValue(sampleMatches);
    const api: Partial<AnnotationsApi> = { findPersonalData };
    const onAddRedactions = vi.fn().mockResolvedValue(undefined);
    const onJumpToPage = vi.fn();
    const onOpenChange = vi.fn();

    render(
      <FindPersonalDataDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={42}
        annotationsApi={api as AnnotationsApi}
        onAddRedactions={onAddRedactions}
        onJumpToPage={onJumpToPage}
      />,
    );

    expect(screen.getByText(t.title)).toBeInTheDocument();
    expect(screen.getByText(t.categoriesLabel)).toBeInTheDocument();
    expect(screen.getByText(t.categories.taiwanId)).toBeInTheDocument();

    // Enable custom keyword
    const keywordCheckbox = screen.getByRole("checkbox", { name: t.customKeywordLabel });
    await userEvent.click(keywordCheckbox);
    const keywordInput = screen.getByPlaceholderText(t.customKeywordPlaceholder);
    expect(keywordInput).toBeEnabled();
    await userEvent.type(keywordInput, "Confidential");

    // Click start search
    const searchBtn = screen.getByRole("button", { name: t.startSearch });
    await userEvent.click(searchBtn);

    expect(findPersonalData).toHaveBeenCalledWith(42, {
      categories: [
        "taiwanId",
        "mobilePhone",
        "landlinePhone",
        "email",
        "creditCard",
        "unifiedBusinessNumber",
        "keyword",
      ],
      keyword: "Confidential",
    });

    // Check results displayed
    expect(await screen.findByText(t.matchesFound(2))).toBeInTheDocument();
    expect(screen.getByText("A12****789")).toBeInTheDocument();
    expect(screen.getByText("0912****78")).toBeInTheDocument();

    // Check jump to page link
    const page2Btn = screen.getByRole("button", { name: t.pageLabel(2) });
    await userEvent.click(page2Btn);
    expect(onJumpToPage).toHaveBeenCalledWith(2);

    // Mark selected items
    const markBtn = screen.getByRole("button", { name: t.markSelected(2) });
    await userEvent.click(markBtn);

    expect(onAddRedactions).toHaveBeenCalledWith(sampleMatches.matches);
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("supports select all / deselect all and marking a subset of matches", async () => {
    const findPersonalData = vi.fn().mockResolvedValue(sampleMatches);
    const api: Partial<AnnotationsApi> = { findPersonalData };
    const onAddRedactions = vi.fn().mockResolvedValue(undefined);
    const onOpenChange = vi.fn();

    render(
      <FindPersonalDataDialog
        open={true}
        onOpenChange={onOpenChange}
        doc={10}
        annotationsApi={api as AnnotationsApi}
        onAddRedactions={onAddRedactions}
        onJumpToPage={vi.fn()}
      />,
    );

    const searchBtn = screen.getByRole("button", { name: t.startSearch });
    await userEvent.click(searchBtn);
    await screen.findByText(t.matchesFound(2));

    // Deselect all
    const deselectBtn = screen.getByRole("button", { name: t.deselectAll });
    await userEvent.click(deselectBtn);

    // Button should now be disabled (0 selected)
    const disabledMarkBtn = screen.getByRole("button", { name: t.markSelected(0) });
    expect(disabledMarkBtn).toBeDisabled();

    // Select all again
    const selectAllBtn = screen.getByRole("button", { name: t.selectAll });
    await userEvent.click(selectAllBtn);

    // Uncheck first match checkbox
    const matchCheckboxes = screen.getAllByRole("checkbox");
    // Find checkboxes for matches (the last two)
    const firstMatchCheckbox = matchCheckboxes[matchCheckboxes.length - 2]!;
    await userEvent.click(firstMatchCheckbox);

    // Mark selected (now 1)
    const markBtn = screen.getByRole("button", { name: t.markSelected(1) });
    await userEvent.click(markBtn);

    expect(onAddRedactions).toHaveBeenCalledWith([sampleMatches.matches[1]!]);
  });

  it("shows caveat banner when hasMore is true", async () => {
    const matchesWithMore: PersonalDataMatches = {
      matches: [sampleMatches.matches[0]!],
      hasMore: true,
    };
    const findPersonalData = vi.fn().mockResolvedValue(matchesWithMore);
    const api: Partial<AnnotationsApi> = { findPersonalData };

    render(
      <FindPersonalDataDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        annotationsApi={api as AnnotationsApi}
        onAddRedactions={vi.fn()}
        onJumpToPage={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: t.startSearch }));
    expect(await screen.findByText(t.hasMoreCaveat)).toBeInTheDocument();
  });

  it("shows error alert on search failure", async () => {
    const findPersonalData = vi.fn().mockRejectedValue(new Error("Worker failure"));
    const api: Partial<AnnotationsApi> = { findPersonalData };

    render(
      <FindPersonalDataDialog
        open={true}
        onOpenChange={vi.fn()}
        doc={1}
        annotationsApi={api as AnnotationsApi}
        onAddRedactions={vi.fn()}
        onJumpToPage={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByRole("button", { name: t.startSearch }));
    expect(await screen.findByRole("alert")).toHaveTextContent(t.failed);
  });
});
