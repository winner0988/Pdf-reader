import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { PresentationMode } from "@/features/viewer/PresentationMode";
import type { PageRenderer } from "@/features/viewer/renderer";
import type { LinkSource } from "@/features/links/source";
import type { PageSize } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";
import type { PageLink } from "@/ipc/generated/contract";

const pages: PageSize[] = [
  { widthPt: 612, heightPt: 792 },
  { widthPt: 612, heightPt: 792 },
  { widthPt: 612, heightPt: 792 },
];

function fakeRenderer(): PageRenderer {
  return {
    render: vi.fn(() => ({
      result: Promise.resolve({
        pixels: new Uint8ClampedArray(4),
        width: 1,
        height: 1,
      }),
      cancel: vi.fn(),
    })),
  };
}

describe("PresentationMode", () => {
  it("has accessible dialog role, title, and live page announcement", () => {
    const renderer = fakeRenderer();
    render(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={2}
        renderer={renderer}
        onPageChange={vi.fn()}
        onClose={vi.fn()}
        onLinkActivate={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog", { name: strings.presentation.title });
    expect(dialog).toBeInTheDocument();
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(screen.getByText(strings.presentation.pageAnnouncement(2, 3))).toBeInTheDocument();
  });

  it("navigates forward with ArrowRight, ArrowDown, PageDown, Space, and click", () => {
    const renderer = fakeRenderer();
    const onPageChange = vi.fn();
    const { rerender } = render(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={1}
        renderer={renderer}
        onPageChange={onPageChange}
        onClose={vi.fn()}
        onLinkActivate={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog");

    fireEvent.keyDown(dialog, { key: "ArrowRight" });
    expect(onPageChange).toHaveBeenLastCalledWith(2);

    fireEvent.keyDown(dialog, { key: "ArrowDown" });
    expect(onPageChange).toHaveBeenLastCalledWith(2);

    fireEvent.keyDown(dialog, { key: "PageDown" });
    expect(onPageChange).toHaveBeenLastCalledWith(2);

    fireEvent.keyDown(dialog, { key: " " });
    expect(onPageChange).toHaveBeenLastCalledWith(2);

    fireEvent.click(dialog);
    expect(onPageChange).toHaveBeenLastCalledWith(2);

    // At the last page, navigating forward does nothing
    rerender(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={3}
        renderer={renderer}
        onPageChange={onPageChange}
        onClose={vi.fn()}
        onLinkActivate={vi.fn()}
      />,
    );
    onPageChange.mockClear();

    fireEvent.keyDown(dialog, { key: "ArrowRight" });
    fireEvent.keyDown(dialog, { key: "ArrowDown" });
    fireEvent.keyDown(dialog, { key: "PageDown" });
    fireEvent.keyDown(dialog, { key: " " });
    fireEvent.click(dialog);
    expect(onPageChange).not.toHaveBeenCalled();
  });

  it("navigates backward with ArrowLeft, ArrowUp, and PageUp", () => {
    const renderer = fakeRenderer();
    const onPageChange = vi.fn();
    const { rerender } = render(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={2}
        renderer={renderer}
        onPageChange={onPageChange}
        onClose={vi.fn()}
        onLinkActivate={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog");

    fireEvent.keyDown(dialog, { key: "ArrowLeft" });
    expect(onPageChange).toHaveBeenLastCalledWith(1);

    fireEvent.keyDown(dialog, { key: "ArrowUp" });
    expect(onPageChange).toHaveBeenLastCalledWith(1);

    fireEvent.keyDown(dialog, { key: "PageUp" });
    expect(onPageChange).toHaveBeenLastCalledWith(1);

    // At the first page, navigating backward does nothing
    rerender(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={1}
        renderer={renderer}
        onPageChange={onPageChange}
        onClose={vi.fn()}
        onLinkActivate={vi.fn()}
      />,
    );
    onPageChange.mockClear();

    fireEvent.keyDown(dialog, { key: "ArrowLeft" });
    fireEvent.keyDown(dialog, { key: "ArrowUp" });
    fireEvent.keyDown(dialog, { key: "PageUp" });
    expect(onPageChange).not.toHaveBeenCalled();
  });

  it("navigates to first and last page with Home and End", () => {
    const renderer = fakeRenderer();
    const onPageChange = vi.fn();
    render(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={2}
        renderer={renderer}
        onPageChange={onPageChange}
        onClose={vi.fn()}
        onLinkActivate={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog");

    fireEvent.keyDown(dialog, { key: "Home" });
    expect(onPageChange).toHaveBeenLastCalledWith(1);

    fireEvent.keyDown(dialog, { key: "End" });
    expect(onPageChange).toHaveBeenLastCalledWith(3);
  });

  it("exits with Escape", () => {
    const renderer = fakeRenderer();
    const onClose = vi.fn();
    render(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={2}
        renderer={renderer}
        onPageChange={vi.fn()}
        onClose={onClose}
        onLinkActivate={vi.fn()}
      />,
    );

    const dialog = screen.getByRole("dialog");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("handles internal and external links on the page", async () => {
    const renderer = fakeRenderer();
    const onPageChange = vi.fn();
    const onLinkActivate = vi.fn();

    const sampleLinks: PageLink[] = [
      {
        id: { pageIndex: 0, index: 0 },
        rect: { x0: 10, y0: 10, x1: 50, y1: 30 },
        target: { kind: "page", pageIndex: 2, x: null, y: null },
      },
      {
        id: { pageIndex: 0, index: 1 },
        rect: { x0: 60, y0: 10, x1: 100, y1: 30 },
        target: { kind: "uri", uri: "https://example.com" },
      },
    ];

    const linkSource: LinkSource = {
      links: vi.fn(() => Promise.resolve(sampleLinks)),
    };

    render(
      <PresentationMode
        doc={1}
        pages={pages}
        rotation={0}
        page={1}
        renderer={renderer}
        links={linkSource}
        onPageChange={onPageChange}
        onClose={vi.fn()}
        onLinkActivate={onLinkActivate}
      />,
    );

    const internalLinkButton = await screen.findByRole("button", { name: strings.links.hoverPage(3) });
    expect(internalLinkButton).toBeInTheDocument();

    // Clicking internal link jumps to that page, and stops propagation so click on overlay does not advance page
    fireEvent.click(internalLinkButton);
    expect(onPageChange).toHaveBeenCalledExactlyOnceWith(3);

    const externalLinkButton = screen.getByRole("button", { name: "https://example.com" });
    fireEvent.click(externalLinkButton);
    expect(onLinkActivate).toHaveBeenCalledExactlyOnceWith(sampleLinks[1]);
  });
});
