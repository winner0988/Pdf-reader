import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
import { Thumbnails, type PageEditing, type SavePages } from "@/features/thumbnails/Thumbnails";
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
    const list = screen.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
    return { ...utils, renderer, onJumpToPage, list, pages, user: userEvent.setup() };
  }

  it("shows only the thumbnails near the view and renders them small and unturned", () => {
    const { renderer } = setup();
    const shown = screen.getAllByRole("option").map((option) => option.getAttribute("aria-label"));
    expect(shown[0]).toBe(strings.canvas.page(1));
    expect(shown.length).toBeLessThan(15);
    const args = renderer.render.mock.calls.map(([call]) => call);
    expect(args.every((call) => call.rotation === "none" && call.doc === 3 && call.scale < 1)).toBe(true);
    expect(args.map((call) => call.pageIndex)).toEqual(shown.map((_, index) => index));
  });

  it("goes to a page, marks the page being read and keeps it in view", async () => {
    const { user, onJumpToPage, rerender, list, renderer, pages } = setup();
    await user.click(screen.getByRole("option", { name: strings.canvas.page(2) }));
    expect(onJumpToPage).toHaveBeenCalledWith(2);
    expect(screen.getByRole("option", { name: strings.canvas.page(1) })).toHaveAttribute("aria-current", "page");

    rerender(
      <div style={{ height: 600 }}>
        <Thumbnails pages={pages} doc={3} renderer={renderer} currentPage={40} onJumpToPage={onJumpToPage} requestDelayMs={0} />
      </div>,
    );
    const scroller = list.parentElement!;
    expect(scroller.scrollTop).toBeGreaterThan(0);
    act(() => scroller.dispatchEvent(new Event("scroll")));
    expect(screen.getByRole("option", { name: strings.canvas.page(40) })).toHaveAttribute("aria-current", "page");
  });

  it("moves between thumbnails with the arrow keys", async () => {
    setup();
    const first = screen.getByRole("option", { name: strings.canvas.page(1) });
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowDown" });
    await act(async () => {});
    expect(screen.getByRole("option", { name: strings.canvas.page(2) })).toHaveFocus();
    fireEvent.keyDown(document.activeElement!, { key: "ArrowUp" });
    await act(async () => {});
    expect(first).toHaveFocus();
  });
});

describe("page management in the thumbnails (B2-05)", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  });

  function setup({
    allowed = true,
    count = 10,
    savePages,
    insertFrom,
  }: {
    allowed?: boolean;
    count?: number;
    savePages?: SavePages;
    insertFrom?: PageEditing["insertFrom"];
  } = {}) {
    const apply = vi.fn<PageEditing["apply"]>(() => Promise.resolve());
    const onJumpToPage = vi.fn();
    const pages = Array(count).fill(LETTER);
    render(
      <div style={{ height: 600 }}>
        <Thumbnails
          pages={pages}
          doc={3}
          currentPage={1}
          onJumpToPage={onJumpToPage}
          editing={{ allowed, apply, insertFrom }}
          savePages={savePages}
          requestDelayMs={0}
        />
      </div>,
    );
    const thumb = (number: number) => screen.getByRole("option", { name: strings.canvas.page(number) });
    const selected = () =>
      screen
        .getAllByRole("option")
        .filter((option) => option.getAttribute("aria-selected") === "true")
        .map((option) => option.getAttribute("aria-label"));
    const menu = async (number: number, item: string) => {
      fireEvent.contextMenu(thumb(number));
      return screen.findByRole("menuitem", { name: new RegExp(`^${item}`) });
    };
    return { apply, onJumpToPage, thumb, selected, menu, user: userEvent.setup() };
  }

  it("selects with a click, Ctrl and Shift, and says how many", async () => {
    const { user, thumb, selected, onJumpToPage } = setup();
    await user.click(thumb(2));
    expect(onJumpToPage).toHaveBeenCalledWith(2);
    await user.keyboard("{Control>}");
    await user.click(thumb(4));
    await user.keyboard("{/Control}{Shift>}");
    await user.click(thumb(6));
    await user.keyboard("{/Shift}");
    expect(selected()).toEqual([4, 5, 6].map((n) => strings.canvas.page(n)));
    expect(screen.getByText(strings.pages.selected(3))).toBeInTheDocument();
    // Only a plain click goes to a page.
    expect(onJumpToPage).toHaveBeenCalledTimes(1);
  });

  it("turns, inserts and deletes the selected pages from the context menu", async () => {
    const { user, apply, menu } = setup();
    await user.click(await menu(3, strings.pages.rotateCw));
    expect(apply).toHaveBeenLastCalledWith({ kind: "rotatePages", pages: [2], by: "cw90" });
    await user.click(await menu(3, strings.pages.rotateCcw));
    expect(apply).toHaveBeenLastCalledWith({ kind: "rotatePages", pages: [2], by: "cw270" });
    await user.click(await menu(3, strings.pages.insertBefore));
    expect(apply).toHaveBeenLastCalledWith({ kind: "insertBlankPage", at: 2, like: 2 });
    await user.click(await menu(5, strings.pages.insertAfter));
    expect(apply).toHaveBeenLastCalledWith({ kind: "insertBlankPage", at: 5, like: 4 });
    await user.click(await menu(5, strings.pages.delete));
    expect(apply).toHaveBeenLastCalledWith({ kind: "deletePages", pages: [4] });
  });

  it("takes the pages of another file in before or after the selected page, and selects them (B2-06)", async () => {
    const insertFrom = vi.fn<NonNullable<PageEditing["insertFrom"]>>(() => Promise.resolve(3));
    const { user, selected, menu } = setup({ insertFrom });
    await user.click(await menu(4, strings.pages.insertFileBefore));
    expect(insertFrom).toHaveBeenLastCalledWith(3);
    // The pages that came in are page 4 and the two after it.
    await waitFor(() => expect(selected()).toEqual([4, 5, 6].map((n) => strings.canvas.page(n))));
    // After the page that was pressed (not one of the selected): the place after page 2.
    await user.click(await menu(2, strings.pages.insertFileAfter));
    expect(insertFrom).toHaveBeenLastCalledWith(2);
  });

  it("does nothing when the user closes the dialog, and says why when the file cannot be used (B2-06)", async () => {
    const insertFrom = vi.fn<NonNullable<PageEditing["insertFrom"]>>(() => Promise.resolve(null));
    const { user, selected, menu } = setup({ insertFrom });
    await user.click(await menu(2, strings.pages.insertFileAfter));
    expect(insertFrom).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("alert")).toBeNull();
    // The selection is the page that was pressed, as for any menu.
    expect(selected()).toEqual([strings.canvas.page(2)]);

    const failures = [
      [{ code: "notAllowed", message: "" }, strings.pages.sourceNotAllowed],
      [{ code: "limitExceeded", message: "" }, strings.pages.sourceTooLarge],
      [{ code: "tooLarge", message: "" }, strings.pages.sourceTooLarge],
      [{ code: "corrupted", message: "" }, strings.pages.sourceFailed],
      [{ code: "notPdf", message: "" }, strings.pages.sourceFailed],
    ] as const;
    for (const [error, text] of failures) {
      insertFrom.mockRejectedValueOnce(error);
      await user.click(await menu(2, strings.pages.insertFileBefore));
      expect(await screen.findByRole("alert")).toHaveTextContent(text);
    }
  });

  it("offers the pages of another file only where it can be asked for, and not against the author (B2-06)", async () => {
    const without = setup();
    fireEvent.contextMenu(without.thumb(2));
    await screen.findByRole("menuitem", { name: new RegExp(`^${strings.pages.rotateCw}`) });
    expect(screen.queryByRole("menuitem", { name: new RegExp(strings.pages.insertFileBefore) })).toBeNull();
  });

  it("saves the selected pages as a file of their own from the context menu, when that is offered (B2-06)", async () => {
    const open = vi.fn<SavePages["open"]>();
    const { user, thumb, menu } = setup({ savePages: { allowed: true, open } });
    await user.click(thumb(2));
    await user.keyboard("{Control>}");
    await user.click(thumb(4));
    await user.keyboard("{/Control}");
    await user.click(await menu(4, strings.pages.saveSelected));
    expect(open).toHaveBeenCalledWith([1, 3]);
  });

  it("does not offer saving pages without the option, and disables it when the author forbids copying", async () => {
    const without = setup();
    fireEvent.contextMenu(without.thumb(2));
    await screen.findByRole("menuitem", { name: new RegExp(`^${strings.pages.rotateCw}`) });
    expect(screen.queryByRole("menuitem", { name: new RegExp(strings.pages.saveSelected) })).toBeNull();
  });

  it("disables saving pages, and says why, when the author forbids copying (MVP-19)", async () => {
    const open = vi.fn<SavePages["open"]>();
    const { user, thumb, menu } = setup({ savePages: { allowed: false, open } });
    await user.click(thumb(2));
    const item = await menu(2, strings.pages.saveSelected);
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(strings.permissions.notAllowed);
    await user.click(item);
    expect(open).not.toHaveBeenCalled();
  });

  it("deletes with the Delete key, but never every page", async () => {
    const { user, apply, thumb } = setup({ count: 3 });
    await user.click(thumb(2));
    await user.keyboard("{Delete}");
    expect(apply).toHaveBeenLastCalledWith({ kind: "deletePages", pages: [1] });
    apply.mockClear();
    await user.keyboard("{Control>}a{/Control}{Delete}");
    expect(apply).not.toHaveBeenCalled();
    expect(await screen.findByRole("alert")).toHaveTextContent(strings.pages.keepOne);
  });

  it("deletes the page with the focus when none is selected", async () => {
    const { apply, thumb } = setup();
    thumb(1).focus();
    fireEvent.keyDown(thumb(1), { key: "ArrowDown" });
    await act(async () => {});
    expect(thumb(2)).toHaveFocus();
    fireEvent.keyDown(thumb(2), { key: "Delete" });
    expect(apply).toHaveBeenLastCalledWith({ kind: "deletePages", pages: [1] });
  });

  it("moves the selected pages to a page number from the keyboard", async () => {
    const { user, apply, menu } = setup();
    await user.click(await menu(5, strings.pages.moveTo));
    const dialog = await screen.findByRole("dialog", { name: strings.pages.move.title });
    const number = within(dialog).getByLabelText(strings.pages.move.page);
    await user.clear(number);
    await user.type(number, "11");
    await user.click(within(dialog).getByRole("button", { name: strings.pages.move.confirm }));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(strings.pages.move.outOfRange(10));
    await user.clear(number);
    await user.type(number, "1");
    await user.click(within(dialog).getByRole("button", { name: strings.pages.move.confirm }));
    expect(apply).toHaveBeenLastCalledWith({ kind: "movePages", pages: [4], before: 0 });
  });

  it("drags the selected pages to another place", () => {
    const { apply, thumb } = setup();
    const list = screen.getByRole("listbox", { name: strings.sidebar.thumbnailsTab });
    // In jsdom every element is at 0, 0: list coordinates are the pointer's.
    fireEvent.pointerDown(thumb(5), { pointerId: 1, button: 0, clientX: 60, clientY: 800 });
    fireEvent.pointerMove(list, { pointerId: 1, clientX: 60, clientY: 10 });
    fireEvent.pointerUp(list, { pointerId: 1, clientX: 60, clientY: 10 });
    expect(apply).toHaveBeenLastCalledWith({ kind: "movePages", pages: [4], before: 0 });
  });

  it("asks to save first when there are too many unsaved changes to keep (B2-13)", async () => {
    const { user, apply, thumb } = setup();
    apply.mockRejectedValueOnce({ code: "limitExceeded", message: "" });
    await user.click(thumb(2));
    await user.keyboard("{Delete}");
    expect(await screen.findByRole("alert")).toHaveTextContent(strings.pages.saveFirst);
    apply.mockRejectedValueOnce({ code: "internal", message: "" });
    await user.keyboard("{Delete}");
    expect(await screen.findByText(strings.pages.failed)).toBeInTheDocument();
  });

  it("offers nothing that changes pages when the author forbids it", async () => {
    const { user, apply, menu, thumb } = setup({ allowed: false });
    expect(await menu(2, strings.pages.delete)).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByText(strings.pages.notAllowed)).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(thumb(2));
    await user.keyboard("{Delete}");
    expect(apply).not.toHaveBeenCalled();
  });
});
