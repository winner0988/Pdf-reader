import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  ITEM_GAP,
  LABEL_HEIGHT,
  layoutThumbs,
  MAX_THUMB_HEIGHT,
  scrollToShow,
  THUMB_WIDTH,
  thumbSize,
  visibleThumbs,
} from "@/features/thumbnails/layout";
import { Thumbnails } from "@/features/thumbnails/Thumbnails";
import type { PageRenderer } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";

const LETTER = { widthPt: 612, heightPt: 792 };
const LANDSCAPE = { widthPt: 842, heightPt: 595 };
const STRIP = { widthPt: 100, heightPt: 2000 };

describe("thumbnail layout", () => {
  it("gives every thumbnail one width and the page's shape, within a height", () => {
    expect(thumbSize(LETTER)).toEqual({ width: THUMB_WIDTH, height: (THUMB_WIDTH * 792) / 612 });
    expect(thumbSize(LANDSCAPE).width).toBe(THUMB_WIDTH);
    expect(thumbSize(STRIP)).toEqual({ width: (MAX_THUMB_HEIGHT * 100) / 2000, height: MAX_THUMB_HEIGHT });
  });

  it("stacks thumbnails with their labels and finds the ones in view", () => {
    const layout = layoutThumbs(Array(100).fill(LETTER));
    const pitch = thumbSize(LETTER).height + LABEL_HEIGHT + ITEM_GAP;
    expect(layout.boxes[1]!.top - layout.boxes[0]!.top).toBeCloseTo(pitch);
    // Items 10 to 12 are in view; three more are kept on either side.
    const range = visibleThumbs(layout, layout.boxes[10]!.top, pitch * 2.5);
    expect(range).toEqual({ first: 7, last: 15 });
    expect(visibleThumbs(layoutThumbs([]), 0, 500)).toEqual({ first: 0, last: -1 });
  });

  it("scrolls as little as needed to show a thumbnail", () => {
    const layout = layoutThumbs(Array(100).fill(LETTER));
    const height = 600;
    expect(scrollToShow(layout, 0, 0, height)).toBe(0);
    const below = scrollToShow(layout, 20, 0, height);
    expect(below + height).toBeGreaterThanOrEqual(layout.boxes[20]!.top + layout.boxes[20]!.height);
    expect(scrollToShow(layout, 2, below, height)).toBe(layout.boxes[2]!.top - ITEM_GAP);
  });
});

describe("Thumbnails", () => {
  beforeEach(() => {
    // jsdom has no canvas.
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  });

  function setup(currentPage = 1) {
    const renderer = {
      render: vi.fn<PageRenderer["render"]>(() => ({ result: new Promise(() => {}), cancel: vi.fn() })),
    } satisfies PageRenderer;
    const onJumpToPage = vi.fn();
    const pages = Array(50).fill(LETTER);
    const utils = render(
      <div style={{ height: 600 }}>
        <Thumbnails
          pages={pages}
          doc={3}
          renderer={renderer}
          currentPage={currentPage}
          onJumpToPage={onJumpToPage}
          requestDelayMs={0}
        />
      </div>,
    );
    const list = screen.getByRole("list", { name: strings.sidebar.thumbnailsTab });
    return { ...utils, renderer, onJumpToPage, list, pages, user: userEvent.setup() };
  }

  it("shows only the thumbnails near the view and renders them small and unturned", () => {
    const { renderer } = setup();
    const shown = screen.getAllByRole("button").map((button) => button.getAttribute("aria-label"));
    expect(shown[0]).toBe(strings.canvas.page(1));
    expect(shown.length).toBeLessThan(15);
    const args = renderer.render.mock.calls.map(([call]) => call);
    expect(args.every((call) => call.rotation === "none" && call.doc === 3 && call.scale < 1)).toBe(true);
    expect(args.map((call) => call.pageIndex)).toEqual(shown.map((_, index) => index));
  });

  it("goes to a page, marks the page being read and keeps it in view", async () => {
    const { user, onJumpToPage, rerender, list, renderer, pages } = setup();
    await user.click(screen.getByRole("button", { name: strings.canvas.page(2) }));
    expect(onJumpToPage).toHaveBeenCalledWith(2);
    expect(screen.getByRole("button", { name: strings.canvas.page(1) })).toHaveAttribute("aria-current", "page");

    rerender(
      <div style={{ height: 600 }}>
        <Thumbnails pages={pages} doc={3} renderer={renderer} currentPage={40} onJumpToPage={onJumpToPage} requestDelayMs={0} />
      </div>,
    );
    const scroller = list.parentElement!;
    expect(scroller.scrollTop).toBeGreaterThan(0);
    act(() => scroller.dispatchEvent(new Event("scroll")));
    expect(screen.getByRole("button", { name: strings.canvas.page(40) })).toHaveAttribute("aria-current", "page");
  });

  it("moves between thumbnails with the arrow keys", async () => {
    setup();
    const first = screen.getByRole("button", { name: strings.canvas.page(1) });
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowDown" });
    await act(async () => {});
    expect(screen.getByRole("button", { name: strings.canvas.page(2) })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    await act(async () => {});
    expect(first).toHaveFocus();
  });
});
