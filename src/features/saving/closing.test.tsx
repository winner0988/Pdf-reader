import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import App from "@/App";
import type { OpenApi } from "@/features/open/api";
import type { SavingApi } from "@/features/saving/api";
import type { RenderApi } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentInfo, OpenEvent } from "@/ipc/generated/contract";

const t = strings.saving;

function renderApp(saving: Partial<SavingApi> = {}) {
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
  const savingApi = {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
    ...saving,
  } satisfies SavingApi;
  const renderApi: RenderApi = { renderPage: () => new Promise(() => {}), cancel: () => Promise.resolve() };
  render(<App api={api} renderApi={renderApi} savingApi={savingApi} />);
  return { api, savingApi, push: (event: OpenEvent) => act(() => emit(event)), user: userEvent.setup() };
}

const info = (doc: number, displayName: string, unsaved: boolean): DocumentInfo => ({
  doc,
  displayName,
  pages: [{ widthPt: 612, heightPt: 792 }],
  hasOutline: false,
  security: { findings: [], scanComplete: true },
  permissions: { copy: true, print: true, printHighQuality: true, modify: true, assemble: true },
  unsaved,
  encrypted: false,
  canUndo: false,
  canRedo: false,
});

function open(push: (event: OpenEvent) => void, tab: number, doc: number, name: string, unsaved: boolean) {
  push({ kind: "opening", tab, displayName: name });
  push({ kind: "opened", tab, info: info(doc, name, unsaved) });
}

const tabList = () => screen.getByRole("tablist", { name: strings.tabs.label });
const question = () => screen.findByRole("dialog", { name: t.askTitle });

describe("unsaved changes (B2-02)", () => {
  it("marks the tab, and closing it asks first", async () => {
    const { api, push, user } = renderApp();
    open(push, 1, 9, "a.pdf", true);
    expect(within(tabList()).getByRole("tab", { name: `a.pdf${strings.tabs.unsaved}` })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: strings.tabs.close("a.pdf") }));
    expect(await question()).toHaveTextContent(t.askOne("a.pdf"));
    await user.click(screen.getByRole("button", { name: t.cancel }));
    expect(api.close).not.toHaveBeenCalled();
    expect(within(tabList()).getAllByRole("tab")).toHaveLength(1);

    await user.keyboard("{Control>}w{/Control}");
    await user.click(within(await question()).getByRole("button", { name: t.discard }));
    expect(api.close).toHaveBeenCalledWith(1);
  });

  it("saves, then closes the tab", async () => {
    const { api, savingApi, push, user } = renderApp();
    open(push, 1, 9, "a.pdf", true);

    await user.click(screen.getByRole("button", { name: strings.tabs.close("a.pdf") }));
    await user.click(within(await question()).getByRole("button", { name: t.save }));
    expect(savingApi.save).toHaveBeenCalledWith(9);
    await waitFor(() => expect(api.close).toHaveBeenCalledWith(1));
  });

  it("keeps the tab when saving fails, and says why", async () => {
    const { api, push, user } = renderApp({
      save: vi.fn(() => Promise.reject({ code: "fileInUse", message: "" })),
    });
    open(push, 1, 9, "a.pdf", true);

    await user.click(screen.getByRole("button", { name: strings.tabs.close("a.pdf") }));
    const dialog = await question();
    await user.click(within(dialog).getByRole("button", { name: t.save }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(strings.error.messages.fileInUse);
    expect(api.close).not.toHaveBeenCalled();
  });

  it("closes a tab without changes at once", async () => {
    const { api, push, user } = renderApp();
    open(push, 1, 9, "a.pdf", false);

    await user.click(screen.getByRole("button", { name: strings.tabs.close("a.pdf") }));
    expect(api.close).toHaveBeenCalledWith(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("asks before the window closes, and saves every changed document", async () => {
    const { savingApi, push, user } = renderApp();
    open(push, 1, 9, "a.pdf", true);
    open(push, 2, 10, "b.pdf", true);
    open(push, 3, 11, "c.pdf", false);

    push({ kind: "closeRequested", tabs: [1, 2] });
    const dialog = await question();
    expect(dialog).toHaveTextContent(t.askMany(2));
    expect(within(dialog).getAllByRole("listitem").map((item) => item.textContent)).toEqual(["a.pdf", "b.pdf"]);
    await user.click(within(dialog).getByRole("button", { name: t.saveAll }));
    await waitFor(() => expect(savingApi.closeWindow).toHaveBeenCalledWith(false));
    expect(savingApi.save).toHaveBeenCalledTimes(2);
    expect(savingApi.save).toHaveBeenNthCalledWith(1, 9);
    expect(savingApi.save).toHaveBeenNthCalledWith(2, 10);
  });

  it("closes the window without saving, or stays", async () => {
    const { savingApi, push, user } = renderApp();
    open(push, 1, 9, "a.pdf", true);

    push({ kind: "closeRequested", tabs: [1] });
    await user.click(within(await question()).getByRole("button", { name: t.cancel }));
    expect(savingApi.closeWindow).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

    push({ kind: "closeRequested", tabs: [1] });
    await user.click(within(await question()).getByRole("button", { name: t.discard }));
    expect(savingApi.closeWindow).toHaveBeenCalledWith(true);
    expect(savingApi.save).not.toHaveBeenCalled();
  });
});
