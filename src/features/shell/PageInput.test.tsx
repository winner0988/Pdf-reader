import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { PageInput } from "@/features/shell/Toolbar";
import { strings } from "@/i18n/zh-TW";

describe("PageInput", () => {
  it("displays physical page number and count without labels", async () => {
    const user = userEvent.setup();
    const onGoToPage = vi.fn();
    render(<PageInput pageCount={10} currentPage={1} onGoToPage={onGoToPage} />);

    const input = screen.getByRole("textbox", { name: strings.toolbar.pageNumber });
    expect(input).toHaveValue("1");
    expect(screen.getByText(strings.toolbar.pageCount(10))).toBeInTheDocument();

    await user.clear(input);
    await user.type(input, "5{Enter}");
    expect(onGoToPage).toHaveBeenCalledWith(5);
  });

  it("shows invalid alert when physical page number is out of range", async () => {
    const user = userEvent.setup();
    const onGoToPage = vi.fn();
    render(<PageInput pageCount={10} currentPage={1} onGoToPage={onGoToPage} />);

    const input = screen.getByRole("textbox", { name: strings.toolbar.pageNumber });
    await user.clear(input);
    await user.type(input, "99{Enter}");
    expect(onGoToPage).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(strings.toolbar.pageOutOfRange(10));
  });

  it("displays custom page label and count with label when label differs from physical page", async () => {
    const user = userEvent.setup();
    const onGoToPage = vi.fn();
    const labels = ["i", "ii", "iii", "iv", "1", "2"];
    render(
      <PageInput
        pageCount={6}
        currentPage={4}
        pageLabels={labels}
        onGoToPage={onGoToPage}
      />,
    );

    const input = screen.getByRole("textbox", {
      name: strings.toolbar.pageNumberWithLabel("iv", 4, 6),
    });
    expect(input).toHaveValue("iv");
    expect(screen.getByText(strings.toolbar.pageCountWithLabel(4, 6))).toBeInTheDocument();

    // Type label 'i' to jump to page 1
    await user.clear(input);
    await user.type(input, "i{Enter}");
    expect(onGoToPage).toHaveBeenCalledWith(1);
  });

  it("matches label first before falling back to physical page number", async () => {
    const user = userEvent.setup();
    const onGoToPage = vi.fn();
    // In this document: page 1..4 are "i".."iv", page 5 is "1", page 6 is "2"
    const labels = ["i", "ii", "iii", "iv", "1", "2"];
    render(
      <PageInput
        pageCount={6}
        currentPage={1}
        pageLabels={labels}
        onGoToPage={onGoToPage}
      />,
    );

    const input = screen.getByRole("textbox");
    // "2" matches label "2" on physical page 6
    await user.clear(input);
    await user.type(input, "2{Enter}");
    expect(onGoToPage).toHaveBeenCalledWith(6);

    // "5" is not a label, so it falls back to physical page 5
    await user.clear(input);
    await user.type(input, "5{Enter}");
    expect(onGoToPage).toHaveBeenCalledWith(5);
  });

  it("notifies when duplicate page labels exist and jumps to first match", async () => {
    const user = userEvent.setup();
    const onGoToPage = vi.fn();
    const onHint = vi.fn();
    const labels = ["intro", "ch1", "intro", "ch2"];
    render(
      <PageInput
        pageCount={4}
        currentPage={1}
        pageLabels={labels}
        onGoToPage={onGoToPage}
        onHint={onHint}
      />,
    );

    const input = screen.getByRole("textbox");
    await user.clear(input);
    await user.type(input, "intro{Enter}");
    expect(onGoToPage).toHaveBeenCalledWith(1);
    expect(onHint).toHaveBeenCalledWith(strings.toolbar.duplicatePageLabel("intro", 1));
  });
});
