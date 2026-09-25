import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import App from "@/App";
import type { OpenApi } from "@/features/open/api";
import type { OutlineApi } from "@/features/outline/useOutline";
import type { RenderApi } from "@/features/viewer/renderer";
import { tabPanelId } from "@/features/tabs/model";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type DocumentInfo, type ErrorCode, type OpenEvent } from "@/ipc/generated/contract";

/** Stands in for the main process: records commands and pushes open events. */
function fakeMainProcess() {
  let emit: (event: OpenEvent) => void = () => {};
  const api = {
    listen: vi.fn((listener: (event: OpenEvent) => void) => {
      emit = listener;
      return () => {};
    }),
    openDialog: vi.fn(() => Promise.resolve(true)),
    retry: vi.fn(() => Promise.resolve()),
    unlock: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
    setActive: vi.fn(() => Promise.resolve()),
  } satisfies OpenApi;
  return { api, push: (event: OpenEvent) => act(() => emit(event)) };
}

const info = (doc: number, displayName: string, pages = 10): DocumentInfo => ({
  doc,
  displayName,
  pages: Array.from({ length: pages }, () => ({ widthPt: 612, heightPt: 792 })),
  hasOutline: false,
  security: { findings: [], scanComplete: true },
});

const opening = (tab: number, displayName: string): OpenEvent => ({ kind: "opening", tab, displayName });
const opened = (tab: number, document: DocumentInfo): OpenEvent => ({ kind: "opened", tab, info: document });
const failed = (code: ErrorCode, displayName = "file.pdf", tab = 1): OpenEvent => ({
  kind: "failed",
  tab,
  displayName,
  error: { code, message: "" },
});

/** Page renders never finish: these tests are about opening, not drawing. */
const idleRenderApi: RenderApi = {
  renderPage: () => new Promise<ArrayBuffer>(() => {}),
  cancel: () => Promise.resolve(),
};

function renderApp() {
  const main = fakeMainProcess();
  const outlineApi = {
    getOutline: vi.fn(() =>
      Promise.resolve({
        items: [{ title: "第 1 章", depth: 0, target: { kind: "page" as const, pageIndex: 2, x: null, y: null } }],
        truncated: false,
      }),
    ),
  } satisfies OutlineApi;
  render(<App api={main.api} renderApi={idleRenderApi} outlineApi={outlineApi} />);
  return { ...main, outlineApi, user: userEvent.setup() };
}

const tabList = () => screen.getByRole("tablist", { name: strings.tabs.label });
/** The status bar of the tab that is shown (hidden tabs are not in the accessibility tree). */
const statusBar = () => screen.getByRole("contentinfo");

describe("App", () => {
  it("starts in the empty state, without tabs", () => {
    renderApp();

    expect(screen.getByRole("heading", { level: 1, name: strings.empty.title })).toBeInTheDocument();
    expect(screen.getByText(strings.empty.privacyNote)).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("opens the native dialog from the button and from Ctrl+O", async () => {
    const { api, user } = renderApp();

    await user.click(screen.getByRole("button", { name: strings.empty.openButton }));
    await user.keyboard("{Control>}o{/Control}");
    expect(api.openDialog).toHaveBeenCalledTimes(2);
  });

  it("shows an opened document in its own tab and names it to the main process", () => {
    const { api, push } = renderApp();

    push(opening(1, "multi-page-10.pdf"));
    expect(within(tabList()).getByRole("tab", { name: /multi-page-10\.pdf/ })).toHaveAttribute("aria-selected", "true");
    push(opened(1, info(9, "multi-page-10.pdf")));

    expect(statusBar()).toHaveTextContent("multi-page-10.pdf");
    expect(statusBar()).toHaveTextContent(/1 \/ 10/);
    expect(api.setActive).toHaveBeenLastCalledWith(1);
  });

  it("keeps several documents in tabs and switches between them", async () => {
    const { api, push, user } = renderApp();

    push(opening(1, "a.pdf"));
    push(opened(1, info(9, "a.pdf", 3)));
    push(opening(2, "b.pdf"));
    push(opened(2, info(10, "b.pdf", 5)));
    // The newest file is shown.
    expect(statusBar()).toHaveTextContent("b.pdf");
    expect(within(tabList()).getAllByRole("tab")).toHaveLength(2);

    await user.click(within(tabList()).getByRole("tab", { name: "a.pdf" }));
    expect(statusBar()).toHaveTextContent("a.pdf");
    expect(statusBar()).toHaveTextContent(/1 \/ 3/);
    expect(api.setActive).toHaveBeenLastCalledWith(1);

    await user.keyboard("{Control>}{Tab}{/Control}");
    expect(statusBar()).toHaveTextContent("b.pdf");
    await user.keyboard("{Control>}{Shift>}{Tab}{/Shift}{/Control}");
    expect(statusBar()).toHaveTextContent("a.pdf");
  });

  it("keeps each tab's view while another tab is shown", async () => {
    const { push, user } = renderApp();
    push(opening(1, "a.pdf"));
    push(opened(1, info(9, "a.pdf")));
    push(opening(2, "b.pdf"));
    push(opened(2, info(10, "b.pdf")));
    const tab = (name: string) => within(tabList()).getByRole("tab", { name });

    await user.click(tab("a.pdf"));
    await user.click(screen.getByRole("button", { name: strings.toolbar.zoomIn }));
    expect(statusBar()).toHaveTextContent("110%");

    await user.click(tab("b.pdf"));
    expect(statusBar()).toHaveTextContent(strings.toolbar.fitWidth);
    await user.click(tab("a.pdf"));
    expect(statusBar()).toHaveTextContent("110%");
  });

  it("asks for an encrypted file's password in its tab and passes it on", async () => {
    const { api, push, user } = renderApp();
    push(opening(1, "a.pdf"));
    push(opened(1, info(9, "a.pdf")));
    push(opening(2, "機密.pdf"));
    push({ kind: "passwordNeeded", tab: 2, displayName: "機密.pdf", wrong: false });

    expect(within(tabList()).getByRole("tab", { selected: true })).toHaveAccessibleName(
      `${strings.tabs.locked}機密.pdf`,
    );
    await user.type(screen.getByLabelText(strings.password.label), "user{Enter}");
    expect(api.unlock).toHaveBeenCalledExactlyOnceWith(2, "user");

    // Cancelling closes the tab and shows the other one.
    await user.click(screen.getByRole("button", { name: strings.password.cancel }));
    expect(api.close).toHaveBeenCalledWith(2);
    expect(statusBar()).toHaveTextContent("a.pdf");
  });

  it("gives the elements of different tabs different ids", async () => {
    const { push, user } = renderApp();
    const risky = (doc: number, name: string): DocumentInfo => ({
      ...info(doc, name),
      security: { findings: [{ kind: "javaScript", count: 1 }], scanComplete: true },
    });
    push(opening(1, "a.pdf"));
    push(opened(1, risky(9, "a.pdf")));
    push(opening(2, "b.pdf"));
    push(opened(2, risky(10, "b.pdf")));
    // The blocked content details, open in both tabs.
    await user.click(screen.getByRole("button", { name: strings.banner.details }));
    await user.click(within(tabList()).getByRole("tab", { name: "a.pdf" }));
    await user.click(screen.getByRole("button", { name: strings.banner.details }));

    const ids = Array.from(document.querySelectorAll("[id]"), (element) => element.id);
    expect(ids.length).toBeGreaterThan(4);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("moves between the regions of the shown tab with F6, whichever tab it is", async () => {
    const { push, user } = renderApp();
    push(opening(1, "a.pdf"));
    push(opened(1, info(9, "a.pdf")));
    push(opening(2, "b.pdf"));
    push(opened(2, info(10, "b.pdf")));

    await user.keyboard("{F6}");
    const focused = document.activeElement as HTMLElement;
    expect(focused.closest("[data-region]")?.getAttribute("data-region")).toBe("toolbar");
    // In the second tab's own toolbar, not the hidden first tab's.
    expect(focused.closest("[role=tabpanel]")).toHaveAttribute("id", tabPanelId(2));
  });

  it.each([
    ["encrypted", strings.error.messages.encrypted],
    ["notPdf", strings.error.messages.notPdf],
    ["corrupted", strings.error.messages.corrupted],
    ["unreadable", strings.error.messages.unreadable],
    ["tooLarge", strings.error.messages.tooLarge],
  ] as const)("shows the %s error in the file's tab without crashing", (code, message) => {
    const { push } = renderApp();

    push(failed(code, "問題檔案.pdf"));

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(strings.error.title);
    expect(alert).toHaveTextContent(message);
    expect(alert).toHaveTextContent("問題檔案.pdf");
    expect(within(tabList()).getByRole("tab", { name: /問題檔案\.pdf/ })).toBeInTheDocument();
  });

  it("retries a retryable failure in the same tab", async () => {
    const { api, push, user } = renderApp();

    push(failed("workerCrashed", "file.pdf", 4));
    await user.click(screen.getByRole("button", { name: strings.error.retry }));
    expect(api.retry).toHaveBeenCalledWith(4);
  });

  it("closes the shown tab with Ctrl+W and shows the one next to it", async () => {
    const { api, push, user } = renderApp();
    push(opened(1, info(9, "a.pdf")));
    push(opening(2, "b.pdf"));
    push(opened(2, info(10, "b.pdf")));

    // Only the shown tab reacts: the hidden tab's reader handles no keys.
    await user.keyboard("{Control>}w{/Control}");
    expect(api.close).toHaveBeenCalledTimes(1);
    expect(api.close).toHaveBeenCalledWith(2);
    expect(statusBar()).toHaveTextContent("a.pdf");

    await user.click(screen.getByRole("button", { name: strings.tabs.close("a.pdf") }));
    expect(api.close).toHaveBeenLastCalledWith(1);
    expect(screen.getByRole("heading", { level: 1, name: strings.empty.title })).toBeInTheDocument();
    expect(api.setActive).toHaveBeenLastCalledWith(null);
  });

  it("ignores events that arrive for a tab the user already closed", async () => {
    const { push, user } = renderApp();
    push(opening(1, "slow.pdf"));
    await user.click(screen.getByRole("button", { name: strings.tabs.close("slow.pdf") }));

    push(opened(1, info(9, "slow.pdf")));
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("says when files were not opened because of the tab limit", async () => {
    const { push, user } = renderApp();

    push({ kind: "tabLimit", ignoredFiles: 2 });
    expect(screen.getByRole("status")).toHaveTextContent(strings.tabs.tabLimit(LIMITS.maxTabs, 2));

    await user.click(screen.getByRole("button", { name: strings.open.dismissNotice }));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("marks the canvas as a drop target while files are dragged over it", () => {
    const { push } = renderApp();
    const canvas = screen.getByRole("main");

    push({ kind: "dragHover", active: true });
    expect(canvas).toHaveAttribute("data-drop-active");
    push({ kind: "dragHover", active: false });
    expect(canvas).not.toHaveAttribute("data-drop-active");
  });

  it("shows an internal error if the dialog command fails", async () => {
    const { api, user } = renderApp();
    api.openDialog.mockRejectedValueOnce({ code: "internal", message: "dialog failed" });

    await user.click(screen.getByRole("button", { name: strings.empty.openButton }));
    expect(await screen.findByRole("alert")).toHaveTextContent(strings.error.messages.internal);
  });

  it("loads the outline of a document that has one", async () => {
    const { outlineApi, push } = renderApp();
    push(opened(1, { ...info(9, "a.pdf"), hasOutline: true }));
    expect(await screen.findByRole("treeitem", { name: "第 1 章" })).toBeInTheDocument();
    expect(outlineApi.getOutline).toHaveBeenCalledWith(9);
  });

  it("does not ask for an outline the document does not have", () => {
    const { outlineApi, push } = renderApp();
    push(opened(1, info(9, "a.pdf")));
    expect(screen.getByText(strings.sidebar.outlineEmpty)).toBeInTheDocument();
    expect(outlineApi.getOutline).not.toHaveBeenCalled();
  });
});
