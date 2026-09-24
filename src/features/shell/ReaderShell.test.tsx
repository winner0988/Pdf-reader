import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
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
    expect(banner).toHaveTextContent("已封鎖此文件中的 4 項內容");
    await user.click(within(banner).getByRole("button", { name: strings.banner.dismiss }));
    expect(screen.queryByRole("region", { name: strings.banner.label })).not.toBeInTheDocument();
  });

  it("has no banner when nothing was blocked", () => {
    renderShell({ kind: "open", document: { ...demoDocument, findings: [] } });
    expect(screen.queryByRole("region", { name: strings.banner.label })).not.toBeInTheDocument();
  });

  it("jumps to a page from the outline and from the page field", async () => {
    const { user } = renderShell(openState);

    await user.click(screen.getByRole("button", { name: "第 2 章 方法" }));
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
});
