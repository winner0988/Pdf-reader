import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { SearchApi } from "@/features/search/useSearch";
import { strings } from "@/i18n/zh-TW";
import type { LinksApi } from "@/features/links/source";
import type { TextApi } from "@/features/text/source";
import { contentWidth, layoutPages, pageLeft } from "@/features/viewer/layout";
import type { OutlineView } from "@/features/outline/tree";
import type { LinkPreview, PageLink, PageText, SearchEvent } from "@/ipc/generated/contract";
import { mediaQuery } from "@/test/setup";

const openState: ShellState = { kind: "open", document: demoDocument };

function renderShell(state: ShellState, props: Partial<Parameters<typeof ReaderShell>[0]> = {}) {
  const onOpen = vi.fn();
  const utils = render(<ReaderShell state={state} onOpen={onOpen} loadingDelayMs={0} {...props} />);
  return { onOpen, user: userEvent.setup(), ...utils };
}

const statusText = () => screen.getByRole("contentinfo").textContent ?? "";

describe("states", () => {
  it("empty: offers to open a file and states the privacy promise", async () => {
    const { onOpen, user } = renderShell({ kind: "empty" });

    expect(screen.getByRole("heading", { name: strings.empty.title })).toBeInTheDocument();
    expect(screen.getByText(strings.empty.privacyNote)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: strings.empty.openButton }));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("loading: shows the file name once the delay has passed", () => {
    vi.useFakeTimers();
    try {
      render(<ReaderShell state={{ kind: "loading", displayName: "報告.pdf" }} onOpen={vi.fn()} />);
      expect(screen.queryByText(strings.loading("報告.pdf"))).not.toBeInTheDocument();
      act(() => vi.advanceTimersByTime(300));
      expect(screen.getByText(strings.loading("報告.pdf"))).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("error: explains the error and offers retry only when it can help", () => {
    const onRetry = vi.fn();
    const { rerender } = renderShell({ kind: "error", code: "corrupted", displayName: "報告.pdf" }, { onRetry });

    const alert = screen.getByRole("alert");
    expect(within(alert).getByText(strings.error.messages.corrupted)).toBeInTheDocument();
    expect(within(alert).getByRole("button", { name: strings.error.openAnother })).toBeInTheDocument();
    expect(within(alert).queryByRole("button", { name: strings.error.retry })).not.toBeInTheDocument();

    rerender(
      <ReaderShell state={{ kind: "error", code: "workerCrashed" }} onOpen={vi.fn()} onRetry={onRetry} />,
    );
    expect(screen.getByRole("button", { name: strings.error.retry })).toBeInTheDocument();
  });

  it("open: shows toolbar, outline, pages and status", () => {
    renderShell(openState);

    expect(screen.getByRole("toolbar")).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: strings.sidebar.label })).toBeInTheDocument();
    // Virtual scrolling: only the pages near the viewport are mounted (MVP-07).
    const mounted = screen.getAllByRole("img", { name: /^第 \d+ 頁$/ });
    expect(mounted.length).toBeGreaterThan(0);
    expect(mounted.length).toBeLessThan(12);
    expect(mounted[0]).toHaveAccessibleName("第 1 頁");
    expect(screen.queryByRole("img", { name: "第 12 頁" })).not.toBeInTheDocument();
    expect(statusText()).toContain("報告.pdf");
    expect(statusText()).toContain("第 1 / 12 頁 · 符合寬度");
  });
});

describe("open document", () => {
  it("shows the blocked-content banner until dismissed", async () => {
    const { user } = renderShell(openState);

    const banner = screen.getByRole("region", { name: strings.banner.label });
    expect(banner).toHaveTextContent("已封鎖此文件中的 3 項內容：JavaScript 腳本、開檔自動動作、遠端資源引用。");
    await user.click(within(banner).getByRole("button", { name: strings.banner.dismiss }));
    expect(screen.queryByRole("region", { name: strings.banner.label })).not.toBeInTheDocument();
  });

  it("has no banner when nothing was blocked", () => {
    renderShell({ kind: "open", document: { ...demoDocument, findings: [] } });
    expect(screen.queryByRole("region", { name: strings.banner.label })).not.toBeInTheDocument();
  });

  it("the details list every blocked kind with its count, and offer no way to run any of it", async () => {
    const { user } = renderShell({
      kind: "open",
      document: {
        ...demoDocument,
        findings: [
          { kind: "uncReference", count: 1 },
          { kind: "javaScript", count: 2 },
          { kind: "openAction", count: 1 },
        ],
      },
    });

    const detailsButton = screen.getByRole("button", { name: strings.banner.details });
    expect(detailsButton).toHaveAttribute("aria-expanded", "false");
    await user.click(detailsButton);
    const panel = screen.getByRole("complementary", { name: strings.banner.detailsTitle });
    expect(detailsButton).toHaveAttribute("aria-expanded", "true");
    expect(within(panel).getByRole("button", { name: strings.banner.detailsClose })).toHaveFocus();
    expect(within(panel).getByText(strings.banner.detailsNote)).toBeInTheDocument();

    const rows = within(panel).getAllByRole("listitem");
    expect(rows.map((row) => row.getAttribute("data-kind"))).toEqual(["javaScript", "openAction", "uncReference"]);
    expect(rows[0]).toHaveTextContent(`${strings.findings.javaScript.name}2 項${strings.findings.javaScript.description}`);
    // The UNC row stands out: following it can send the user's account hash to a server.
    expect(within(rows[2]!).getByText(strings.findings.uncReference.name)).toHaveClass("text-destructive");
    // Closing is the only thing the panel can do.
    expect(within(panel).getAllByRole("button")).toHaveLength(1);
    expect(within(panel).queryByRole("note")).not.toBeInTheDocument();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("complementary", { name: strings.banner.detailsTitle })).not.toBeInTheDocument();
    expect(detailsButton).toHaveFocus();
  });

  it("an unfinished scan is shown even when nothing was found", async () => {
    const { user } = renderShell({ kind: "open", document: { ...demoDocument, findings: [], scanComplete: false } });

    const banner = screen.getByRole("region", { name: strings.banner.label });
    expect(banner).toHaveTextContent(strings.banner.scanIncomplete);
    await user.click(within(banner).getByRole("button", { name: strings.banner.details }));
    const panel = screen.getByRole("complementary", { name: strings.banner.detailsTitle });
    expect(within(panel).getByRole("note")).toHaveTextContent(strings.banner.scanIncomplete);
    await user.click(within(panel).getByRole("button", { name: strings.banner.detailsClose }));
    expect(screen.queryByRole("complementary", { name: strings.banner.detailsTitle })).not.toBeInTheDocument();
  });

  it("jumps to a page from the outline and from the page field", async () => {
    const { user } = renderShell(openState);

    await user.click(screen.getByRole("treeitem", { name: "第 2 章 方法" }));
    expect(statusText()).toContain("第 5 / 12 頁");

    const pageField = screen.getByRole("textbox", { name: strings.toolbar.pageNumber });
    await user.clear(pageField);
    await user.type(pageField, "99{Enter}");
    expect(screen.getByRole("alert")).toHaveTextContent(strings.toolbar.pageOutOfRange(12));
    expect(statusText()).toContain("第 5 / 12 頁");

    await user.clear(pageField);
    await user.type(pageField, "3{Enter}");
    expect(statusText()).toContain("第 3 / 12 頁");
  });

  it("zooms and rotates from the toolbar", async () => {
    const { user } = renderShell(openState);

    await user.click(screen.getByRole("button", { name: strings.toolbar.zoomIn }));
    expect(statusText()).toContain("110%");
    await user.selectOptions(screen.getByRole("combobox", { name: strings.toolbar.zoomLevel }), "200");
    expect(statusText()).toContain("200%");

    const firstPage = screen.getByRole("img", { name: "第 1 頁" });
    const { width, height } = firstPage.style;
    await user.click(screen.getByRole("button", { name: strings.toolbar.rotateCw }));
    expect(firstPage.style.width).toBe(height);
    expect(firstPage.style.height).toBe(width);
  });

  it("resets the view when another document opens", async () => {
    const { user, rerender } = renderShell(openState);
    await user.click(screen.getByRole("button", { name: strings.toolbar.zoomIn }));
    expect(statusText()).toContain("110%");

    rerender(
      <ReaderShell state={{ kind: "open", document: { ...demoDocument } }} onOpen={vi.fn()} loadingDelayMs={0} />,
    );
    expect(statusText()).toContain("第 1 / 12 頁 · 符合寬度");
  });
});

describe("shortcuts", () => {
  it("Ctrl+O opens a file in any state", async () => {
    const { onOpen, user } = renderShell({ kind: "empty" });
    await user.keyboard("{Control>}o{/Control}");
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("zoom shortcuts change the zoom", async () => {
    const { user } = renderShell(openState);

    await user.keyboard("{Control>}={/Control}");
    expect(statusText()).toContain("110%");
    await user.keyboard("{Control>}-{/Control}{Control>}-{/Control}");
    expect(statusText()).toContain("90%");
    await user.keyboard("{Control>}0{/Control}");
    expect(statusText()).toContain(strings.toolbar.fitPage);
    await user.keyboard("{Control>}1{/Control}");
    expect(statusText()).toContain("100%");
  });

  it("Ctrl+F opens the search bar with focus; Escape closes it", async () => {
    const { user } = renderShell(openState);

    await user.keyboard("{Control>}f{/Control}");
    const field = screen.getByRole("textbox", { name: strings.search.placeholder });
    expect(field).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("search")).not.toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveFocus();
  });

  it("Enter searches, hits are marked on the pages, F3 steps through them, closing clears the marks", async () => {
    let emit: (event: SearchEvent) => void = () => {};
    const searchApi = {
      search: vi.fn((_args, onEvent: (event: SearchEvent) => void) => {
        emit = (event) => act(() => onEvent(event));
        return new Promise<void>(() => {});
      }),
      cancel: vi.fn(() => Promise.resolve()),
    } satisfies SearchApi;
    const { user } = renderShell({ kind: "open", document: { ...demoDocument, doc: 5 } }, { searchApi });
    const line = { ul: { x: 72, y: 72 }, ur: { x: 144, y: 72 }, ll: { x: 72, y: 84 }, lr: { x: 144, y: 84 } };
    const marks = (selector = "polygon") => document.querySelectorAll(`[data-highlights] ${selector}`);

    await user.keyboard("{Control>}f{/Control}needle{Enter}");
    expect(searchApi.search).toHaveBeenCalledWith(
      expect.objectContaining({ doc: 5, query: "needle", caseSensitive: false }),
      expect.any(Function),
    );
    emit({ kind: "hits", pageIndex: 0, hits: [{ quads: [line] }, { quads: [line] }] });
    emit({ kind: "done", totalHits: 2, truncated: false, noTextLayer: false });
    const search = screen.getByRole("search");
    expect(within(search).getByRole("status")).toHaveTextContent(strings.search.count(1, 2));
    expect(marks()).toHaveLength(3); // two hits, one of them outlined
    expect(marks("[data-current]")).toHaveLength(1);

    // F3 works from the search field too.
    await user.keyboard("{F3}");
    expect(within(search).getByRole("status")).toHaveTextContent(strings.search.count(2, 2));
    await user.keyboard("{Shift>}{F3}{/Shift}");
    expect(within(search).getByRole("status")).toHaveTextContent(strings.search.count(1, 2));

    await user.keyboard("{Escape}");
    expect(marks()).toHaveLength(0);
    // F3 reopens the bar and searches again.
    await user.keyboard("{F3}");
    expect(screen.getByRole("search")).toBeInTheDocument();
    expect(searchApi.search).toHaveBeenCalledTimes(2);
  });

  describe("links", () => {
    const pageLinks: PageLink[] = [
      {
        id: { pageIndex: 0, index: 0 },
        rect: { x0: 72, y0: 80, x1: 300, y1: 102 },
        target: { kind: "page", pageIndex: 7, x: null, y: null },
      },
      {
        id: { pageIndex: 0, index: 1 },
        rect: { x0: 72, y0: 120, x1: 300, y1: 142 },
        target: { kind: "uri", uri: "https://example.invalid/docs" },
      },
      {
        id: { pageIndex: 0, index: 2 },
        rect: { x0: 72, y0: 160, x1: 300, y1: 182 },
        target: { kind: "blocked", action: "launch", target: "calc.exe" },
      },
    ];
    const preview: LinkPreview = {
      uri: "https://example.invalid/docs",
      opens: "https://example.invalid/docs",
      host: "example.invalid",
      asciiHost: null,
    };
    const setup = () => {
      const linksApi = {
        getPageLinks: vi.fn((_doc: number, pageIndex: number) => Promise.resolve(pageIndex === 0 ? pageLinks : [])),
        describeLink: vi.fn(() => Promise.resolve(preview)),
        openLink: vi.fn(() => Promise.resolve()),
        describeOutlineLink: vi.fn(() => Promise.resolve(preview)),
        openOutlineLink: vi.fn(() => Promise.resolve()),
      } satisfies LinksApi;
      const outline: OutlineView = {
        status: "ready",
        truncated: false,
        items: [
          { title: "第 1 章", depth: 0, target: { kind: "page", pageIndex: 0, x: null, y: null } },
          { title: "官方網站", depth: 0, target: { kind: "uri", uri: "https://example.invalid/docs" } },
          { title: "執行程式", depth: 0, target: { kind: "blocked", action: "launch", target: "calc.exe" } },
        ],
      };
      const utils = renderShell({ kind: "open", document: { ...demoDocument, doc: 5 } }, { linksApi, outline });
      return { ...utils, linksApi };
    };

    it("an internal link jumps to its page; the status bar says what a link does", async () => {
      const { user, linksApi } = setup();

      const toPage = await screen.findByRole("button", { name: "前往第 8 頁" });
      await user.hover(toPage);
      expect(statusText()).toContain("前往第 8 頁");
      await user.click(toPage);
      expect(statusText()).toContain("第 8 / 12 頁");
      expect(linksApi.getPageLinks).toHaveBeenCalledWith(5, 0);
    });

    it("a web link opens only after confirmation, named by its id", async () => {
      const { user, linksApi } = setup();

      await user.click(await screen.findByRole("button", { name: "https://example.invalid/docs" }));
      const dialog = await screen.findByRole("dialog", { name: strings.links.confirmTitle });
      expect(linksApi.describeLink).toHaveBeenCalledWith(5, { pageIndex: 0, index: 1 });
      expect(within(dialog).getByText("example.invalid")).toHaveAttribute("data-host");
      expect(within(dialog).getByLabelText(strings.links.confirmFullUrl)).toHaveTextContent("https://example.invalid/docs");
      // Cancel is where the focus starts; Escape cancels.
      await waitFor(() => expect(within(dialog).getByRole("button", { name: strings.links.cancel })).toHaveFocus());
      await user.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      expect(linksApi.openLink).not.toHaveBeenCalled();

      await user.click(screen.getByRole("button", { name: "https://example.invalid/docs" }));
      await user.click(await screen.findByRole("button", { name: strings.links.open }));
      expect(linksApi.openLink).toHaveBeenCalledWith(5, { pageIndex: 0, index: 1 });
      await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    });

    it("says so when the system cannot open it, and stays open", async () => {
      const { user, linksApi } = setup();
      linksApi.openLink.mockRejectedValueOnce({ code: "internal", message: "" });

      await user.click(await screen.findByRole("button", { name: "https://example.invalid/docs" }));
      await user.click(await screen.findByRole("button", { name: strings.links.open }));
      expect(await screen.findByRole("alert")).toHaveTextContent(strings.links.openFailed);
      expect(screen.getByRole("dialog", { name: strings.links.confirmTitle })).toBeInTheDocument();
    });

    it("an outline item with a web link is confirmed and opened by its position (#49)", async () => {
      const { user, linksApi } = setup();

      await user.click(screen.getByRole("treeitem", { name: /官方網站/ }));
      const dialog = await screen.findByRole("dialog", { name: strings.links.confirmTitle });
      expect(linksApi.describeOutlineLink).toHaveBeenCalledWith(5, 1);
      await user.click(within(dialog).getByRole("button", { name: strings.links.open }));
      expect(linksApi.openOutlineLink).toHaveBeenCalledWith(5, 1);
      expect(linksApi.openLink).not.toHaveBeenCalled();
      await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());

      // A blocked outline item explains itself, like a blocked link on a page.
      await user.click(screen.getByRole("treeitem", { name: /執行程式/ }));
      const blocked = await screen.findByRole("dialog", { name: strings.links.blockedTitle });
      expect(blocked).toHaveTextContent(strings.links.blocked.launch.description);
      expect(linksApi.describeOutlineLink).toHaveBeenCalledTimes(1);
    });

    it("a blocked link only explains why, with nothing to open it", async () => {
      const { user, linksApi } = setup();

      await user.click(await screen.findByRole("button", { name: "已封鎖：啟動外部程式" }));
      const dialog = await screen.findByRole("dialog", { name: strings.links.blockedTitle });
      expect(dialog).toHaveTextContent(strings.links.blocked.launch.description);
      expect(within(dialog).getByLabelText(strings.links.blockedContent)).toHaveTextContent("calc.exe");
      expect(within(dialog).getAllByRole("button").map((button) => button.textContent)).toEqual([
        strings.links.blockedCopy,
        strings.links.close,
      ]);
      await waitFor(() => expect(within(dialog).getByRole("button", { name: strings.links.close })).toHaveFocus());
      await user.click(within(dialog).getByRole("button", { name: strings.links.close }));
      await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      expect(linksApi.describeLink).not.toHaveBeenCalled();
      expect(linksApi.openLink).not.toHaveBeenCalled();
    });
  });

  it("on narrow windows the sidebar starts closed, floats, and closes when the page is used", async () => {
    const original = window.innerWidth;
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 800 });
    try {
      const { user } = renderShell(openState);
      expect(screen.queryByRole("complementary")).not.toBeInTheDocument();

      await user.keyboard("{F4}");
      expect(screen.getByRole("complementary")).toBeInTheDocument();
      await user.pointer({ keys: "[MouseLeft]", target: screen.getByRole("main") });
      expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
    } finally {
      Object.defineProperty(window, "innerWidth", { configurable: true, value: original });
    }
  });

  it("F4 toggles the sidebar", async () => {
    const { user } = renderShell(openState);

    await user.keyboard("{F4}");
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
    await user.keyboard("{F4}");
    expect(screen.getByRole("complementary")).toBeInTheDocument();
  });

  it("Ctrl+/ lists every shortcut", async () => {
    const { user } = renderShell(openState);

    await user.keyboard("{Control>}/{/Control}");
    const dialog = await screen.findByRole("dialog", { name: strings.shortcuts.title });
    expect(within(dialog).getByText(strings.shortcuts.descriptions.open)).toBeInTheDocument();
    expect(within(dialog).getByText("Ctrl+O")).toBeInTheDocument();
  });

  it("document shortcuts do nothing without a document", async () => {
    const { user } = renderShell({ kind: "empty" });
    await user.keyboard("{Control>}f{/Control}");
    expect(screen.queryByRole("search")).not.toBeInTheDocument();
  });
});

describe("selecting and copying text (MVP-15)", () => {
  // "Hello world" 72 pt from the left and 100 pt from the top, 6 pt per character; the second
  // page is scanned (no text).
  const hello: PageText = {
    lines: [
      {
        text: "Hello world",
        quad: { ul: { x: 72, y: 100 }, ur: { x: 138, y: 100 }, ll: { x: 72, y: 112 }, lr: { x: 138, y: 112 } },
        edges: Array.from({ length: 12 }, (_, i) => i * 6),
      },
    ],
    truncated: false,
  };
  const scanned: PageText = { lines: [], truncated: false };

  /** Client coordinates of a point on a page, in page points, at 100% (jsdom puts the view at 0, 0). */
  const at = (index: number, x: number, y: number) => {
    const layout = layoutPages(demoDocument.pages, 0, 1);
    const box = layout.boxes[index]!;
    return {
      clientX: pageLeft(box, contentWidth(layout, 1000)) + (x * 4) / 3,
      clientY: box.top + (y * 4) / 3,
    };
  };

  async function setup() {
    const textApi = {
      getPageText: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? hello : scanned)),
    } satisfies TextApi;
    const utils = renderShell({ kind: "open", document: { ...demoDocument, doc: 5 } }, { textApi });
    // The clipboard stub lives as long as the window: start every test from something known.
    await navigator.clipboard.writeText("before");
    // 100%: a page point is 4/3 CSS pixels.
    await utils.user.keyboard("{Control>}1{/Control}");
    const canvas = screen.getByRole("main");
    Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
    Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 800 });
    act(() => canvas.dispatchEvent(new Event("scroll")));
    // The first two pages' text is asked for once they have been in view for a moment.
    await waitFor(() => expect(textApi.getPageText).toHaveBeenCalledWith(5, 1));
    await act(async () => {});
    const pages = screen.getByRole("img", { name: "第 1 頁" }).parentElement!;
    const selectHello = () => {
      fireEvent.mouseDown(pages, { button: 0, detail: 1, ...at(0, 73, 106) });
      fireEvent.mouseMove(window, at(0, 101, 106));
      fireEvent.mouseUp(window);
    };
    return { ...utils, pages, selectHello, textApi };
  }

  it("Ctrl+C copies the selected text", async () => {
    const { user, selectHello } = await setup();
    selectHello();
    await user.keyboard("{Control>}c{/Control}");
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe("Hello"));
  });

  it("Ctrl+C in a text field copies the field's text, not the page's", async () => {
    const { user, selectHello } = await setup();
    selectHello();
    await user.click(screen.getByRole("textbox", { name: strings.toolbar.pageNumber }));
    await user.keyboard("{Control>}c{/Control}");
    await act(async () => {});
    expect(await navigator.clipboard.readText()).not.toBe("Hello");
  });

  it("right-clicking the document offers to copy, once something is selected", async () => {
    const { user, pages, selectHello } = await setup();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    expect(await screen.findByRole("menuitem", { name: /複製/ })).toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Escape}");

    selectHello();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    const copy = await screen.findByRole("menuitem", { name: /複製/ });
    expect(copy).not.toHaveAttribute("aria-disabled", "true");
    await user.click(copy);
    await waitFor(async () => expect(await navigator.clipboard.readText()).toBe("Hello"));
  });

  it("says for a while that a scanned page has no text to select", async () => {
    const { pages } = await setup();
    vi.useFakeTimers();
    try {
      fireEvent.mouseDown(pages, { button: 0, detail: 1, ...at(1, 100, 100) });
      fireEvent.mouseMove(window, at(1, 200, 200));
      fireEvent.mouseUp(window);
      expect(statusText()).toContain(strings.text.noTextLayer);
      act(() => vi.advanceTimersByTime(4000));
      expect(statusText()).not.toContain(strings.text.noTextLayer);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("accessibility", () => {
  it("every toolbar control has an accessible name", () => {
    renderShell(openState);

    const toolbar = screen.getByRole("toolbar");
    for (const control of [
      ...within(toolbar).getAllByRole("button"),
      ...within(toolbar).getAllByRole("textbox"),
      ...within(toolbar).getAllByRole("combobox"),
    ]) {
      expect(control).toHaveAccessibleName();
    }
  });

  it("the toolbar can be walked with Tab", async () => {
    const { user } = renderShell(openState);

    await user.tab();
    expect(screen.getByRole("button", { name: strings.toolbar.toggleSidebar })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: strings.toolbar.open })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("textbox", { name: strings.toolbar.pageNumber })).toHaveFocus();
  });

  it("F6 moves focus between regions", async () => {
    const { user } = renderShell(openState);

    await user.keyboard("{F6}");
    expect(screen.getByRole("button", { name: strings.toolbar.toggleSidebar })).toHaveFocus();
    await user.keyboard("{F6}");
    expect(document.activeElement?.closest("[data-region]")?.getAttribute("data-region")).toBe("banner");
    await user.keyboard("{F6}");
    expect(document.activeElement?.closest("[data-region]")?.getAttribute("data-region")).toBe("sidebar");
  });
});

describe("theme", () => {
  it("follows the system by default", () => {
    mediaQuery.matches = true;
    renderShell(openState);
    expect(document.documentElement).toHaveClass("dark");
  });

  it("light overrides a dark system theme", async () => {
    mediaQuery.matches = true;
    const { user } = renderShell(openState);

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitemradio", { name: strings.menu.themeLight }));
    expect(document.documentElement).not.toHaveClass("dark");
  });

  it("dark overrides a light system theme", async () => {
    const { user } = renderShell(openState);
    expect(document.documentElement).not.toHaveClass("dark");

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitemradio", { name: strings.menu.themeDark }));
    expect(document.documentElement).toHaveClass("dark");
  });

  it("about states the privacy promise", async () => {
    const { user } = renderShell(openState);

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: strings.menu.about }));
    const dialog = await screen.findByRole("dialog", { name: strings.about.title });
    for (const line of strings.about.privacy) {
      expect(within(dialog).getByText(line)).toBeInTheDocument();
    }
    // Screen-reader text comes from the string table too.
    expect(within(dialog).getByRole("button", { name: strings.close })).toBeInTheDocument();
  });

  it("set as default opens Windows Settings through the main process", async () => {
    const systemApi = { openDefaultAppsSettings: vi.fn(() => Promise.resolve()) };
    const { user } = renderShell({ kind: "empty" }, { systemApi });

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: strings.menu.setDefault }));

    expect(systemApi.openDefaultAppsSettings).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog", { name: strings.defaultApp.failedTitle })).not.toBeInTheDocument();
  });

  it("explains how to set the default by hand when Settings cannot be opened", async () => {
    const systemApi = { openDefaultAppsSettings: vi.fn(() => Promise.reject(new Error("no settings"))) };
    const { user } = renderShell({ kind: "empty" }, { systemApi });

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: strings.menu.setDefault }));

    const dialog = await screen.findByRole("dialog", { name: strings.defaultApp.failedTitle });
    expect(within(dialog).getByText(strings.defaultApp.failedHelp)).toBeInTheDocument();
  });
});
