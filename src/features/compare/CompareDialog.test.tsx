import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { CompareApi } from "@/features/compare/api";
import { CompareDialog, type CompareTabOption } from "@/features/compare/CompareDialog";
import { strings } from "@/i18n/zh-TW";
import type { CompareTextResult, ExportCompareTextOutcome } from "@/ipc/generated/contract";

const t = strings.compare;

const mockTabs: CompareTabOption[] = [
  { tab: 2, doc: 20, displayName: "Doc-B.pdf", copyAllowed: true },
  { tab: 3, doc: 30, displayName: "Doc-C.pdf", copyAllowed: true },
];

function createMockApi(
  compareResult: CompareTextResult | Error,
  exportOutcome: ExportCompareTextOutcome | Error = { kind: "written", count: 2, truncated: false },
): CompareApi {
  return {
    compareText: vi.fn(() =>
      compareResult instanceof Error ? Promise.reject(compareResult) : Promise.resolve(compareResult),
    ),
    exportCompareText: vi.fn(() =>
      exportOutcome instanceof Error ? Promise.reject(exportOutcome) : Promise.resolve(exportOutcome),
    ),
  };
}

describe("CompareDialog (B3-24, E9)", () => {
  it("renders other tabs in dropdown and runs compare on click", async () => {
    const user = userEvent.setup();
    const result: CompareTextResult = {
      items: [
        {
          pageA: 0,
          pageB: 0,
          deleted: "Hello world",
          inserted: "Hello there world",
          truncated: false,
        },
      ],
      identical: false,
      empty: false,
      emptyA: false,
      emptyB: false,
      recognised: false,
      pageTextTruncated: false,
      truncated: false,
    };
    const api = createMockApi(result);
    const onJumpToPage = vi.fn();
    const onOpenChange = vi.fn();

    render(
      <CompareDialog
        open={true}
        onOpenChange={onOpenChange}
        currentDoc={10}
        otherTabs={mockTabs}
        api={api}
        onJumpToPage={onJumpToPage}
      />,
    );

    expect(screen.getByRole("heading", { name: t.title })).toBeInTheDocument();
    const select = screen.getByLabelText(t.chooseTab);
    expect(select).toHaveValue("20");

    // Select Doc-C
    await user.selectOptions(select, "30");
    expect(select).toHaveValue("30");

    // Click start compare
    await user.click(screen.getByRole("button", { name: t.start }));

    expect(api.compareText).toHaveBeenCalledWith({ docA: 10, docB: 30 });

    // Differences list appears
    expect(await screen.findByText(t.diffCount(1))).toBeInTheDocument();
    expect(screen.getByText("Hello world")).toBeInTheDocument();
    expect(screen.getByText("Hello there world")).toBeInTheDocument();

    // Clicking diff item triggers jump to page (1-based)
    await user.click(screen.getByRole("button", { name: new RegExp(t.itemPages(1, 1)) }));
    expect(onJumpToPage).toHaveBeenCalledWith(1);
  });

  it("shows identical document state", async () => {
    const user = userEvent.setup();
    const result: CompareTextResult = {
      items: [],
      identical: true,
      empty: false,
      emptyA: false,
      emptyB: false,
      recognised: false,
      pageTextTruncated: false,
      truncated: false,
    };
    const api = createMockApi(result);

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        otherTabs={mockTabs}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));
    expect(await screen.findByText(t.identical)).toBeInTheDocument();
  });

  it("shows empty text state", async () => {
    const user = userEvent.setup();
    const result: CompareTextResult = {
      items: [],
      identical: false,
      empty: true,
      emptyA: true,
      emptyB: true,
      recognised: false,
      pageTextTruncated: false,
      truncated: false,
    };
    const api = createMockApi(result);

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        otherTabs={mockTabs}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));
    expect(await screen.findByText(t.noText)).toBeInTheDocument();
  });

  it("shows notice when only one side has no text", async () => {
    const user = userEvent.setup();
    const resultAEmpty: CompareTextResult = {
      items: [
        {
          pageA: 0,
          pageB: 0,
          deleted: "",
          inserted: "New clauses",
          truncated: false,
        },
      ],
      identical: false,
      empty: false,
      emptyA: true,
      emptyB: false,
      recognised: false,
      pageTextTruncated: false,
      truncated: false,
    };
    const api = createMockApi(resultAEmpty);

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        currentDisplayName="Doc-A.pdf"
        otherTabs={mockTabs}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));
    expect(await screen.findByText(t.noTextOneSide("Doc-A.pdf"))).toBeInTheDocument();
  });

  it("shows localized fallback labels when display names are missing", async () => {
    const user = userEvent.setup();
    const resultBothEmpty: CompareTextResult = {
      items: [
        {
          pageA: 0,
          pageB: 0,
          deleted: "",
          inserted: "diff",
          truncated: false,
        },
      ],
      identical: false,
      empty: false,
      emptyA: true,
      emptyB: true,
      recognised: false,
      pageTextTruncated: false,
      truncated: false,
    };
    const api = createMockApi(resultBothEmpty);
    const tabsWithoutName: CompareTabOption[] = [
      { tab: 2, doc: 20, displayName: "", copyAllowed: true },
    ];

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        otherTabs={tabsWithoutName}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));
    expect(await screen.findByText(t.noTextOneSide(t.currentTab))).toBeInTheDocument();
    expect(screen.getByText(t.noTextOneSide(t.targetTabDefault))).toBeInTheDocument();
  });

  it("disables tabs that do not allow copying and shows explanation", async () => {
    const tabsWithRestrictions: CompareTabOption[] = [
      { tab: 2, doc: 20, displayName: "Doc-Allowed.pdf", copyAllowed: true },
      { tab: 3, doc: 30, displayName: "Doc-Restricted.pdf", copyAllowed: false },
    ];
    const api = createMockApi({
      items: [],
      identical: true,
      empty: false,
      emptyA: false,
      emptyB: false,
      recognised: false,
      pageTextTruncated: false,
      truncated: false,
    });

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        otherTabs={tabsWithRestrictions}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    const restrictedOption = screen.getByRole("option", { name: /Doc-Restricted/ });
    expect(restrictedOption).toBeDisabled();
    expect(restrictedOption).toHaveTextContent(t.tabNoCopy);
  });

  it("disables comparison and explains why when all other tabs cannot be copied", async () => {
    const tabsAllRestricted: CompareTabOption[] = [
      { tab: 3, doc: 30, displayName: "Doc-Restricted.pdf", copyAllowed: false },
    ];
    const api = createMockApi({
      items: [],
      identical: true,
      empty: false,
      emptyA: false,
      emptyB: false,
      recognised: false,
      pageTextTruncated: false,
      truncated: false,
    });

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        otherTabs={tabsAllRestricted}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: t.start })).toBeDisabled();
    expect(screen.getByText(t.allTabsNoCopy)).toBeInTheDocument();
  });

  it("exports results to plain text and markdown", async () => {
    const user = userEvent.setup();
    const result: CompareTextResult = {
      items: [
        {
          pageA: 1,
          pageB: 1,
          deleted: "old clause",
          inserted: "new clause",
          truncated: true,
        },
      ],
      identical: false,
      empty: false,
      emptyA: false,
      emptyB: false,
      recognised: true,
      pageTextTruncated: true,
      truncated: true,
    };
    const api = createMockApi(result, { kind: "written", count: 1, truncated: false });

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        otherTabs={mockTabs}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));
    expect(await screen.findByText(t.truncated)).toBeInTheDocument();
    expect(screen.getByText(t.hasRecognised)).toBeInTheDocument();
    expect(screen.getByText(t.pageTextTruncated)).toBeInTheDocument();

    // Export text
    await user.click(screen.getByRole("button", { name: t.exportText }));
    expect(api.exportCompareText).toHaveBeenCalledWith({ docA: 10, docB: 20, format: "text" });
    expect(await screen.findByText(t.exportDone(1))).toBeInTheDocument();

    // Export markdown
    await user.click(screen.getByRole("button", { name: t.exportMarkdown }));
    expect(api.exportCompareText).toHaveBeenCalledWith({ docA: 10, docB: 20, format: "markdown" });
  });

  it("displays compare error when backend fails", async () => {
    const user = userEvent.setup();
    const api: CompareApi = {
      compareText: vi.fn().mockRejectedValue({
        code: "invalidArgument",
        message: "the document's author does not allow copying its content",
      }),
      exportCompareText: vi.fn(),
    };

    render(
      <CompareDialog
        open={true}
        onOpenChange={vi.fn()}
        currentDoc={10}
        otherTabs={mockTabs}
        api={api}
        onJumpToPage={vi.fn()}
      />,
    );

    await user.click(screen.getByRole("button", { name: t.start }));
    expect(await screen.findByRole("alert")).toHaveTextContent(t.copyNotAllowed);
  });
});
