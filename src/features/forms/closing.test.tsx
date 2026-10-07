import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import App from "@/App";
import type { FormsApi } from "@/features/forms/source";
import type { OpenApi } from "@/features/open/api";
import type { SavingApi } from "@/features/saving/api";
import type { EditingApi } from "@/features/thumbnails/api";
import type { RenderApi } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentInfo, FormField, OpenEvent } from "@/ipc/generated/contract";

// Closing a tab while a value is being typed in a field of its form (B2-09): the value is a change
// of the document, so it is sent first, and the tab asks whether to save it.

const FIELD: FormField = {
  id: 6,
  group: 6,
  kind: "text",
  rect: { x0: 72, y0: 100, x1: 300, y1: 124 },
  label: "Your name",
  value: "Jane",
  onValue: null,
  options: [],
  readOnly: false,
  required: false,
  multiline: false,
  password: false,
  editable: false,
  multiSelect: false,
  maxLen: null,
  hasScript: false,
};

const info = (doc: number, unsaved: boolean): DocumentInfo => ({
  doc,
  displayName: "form.pdf",
  pages: [{ widthPt: 612, heightPt: 792 }],
  hasOutline: false,
  hasForm: true,
  security: { findings: [], scanComplete: true },
  permissions: { copy: true, print: true, printHighQuality: true, modify: true, assemble: true, annotate: true, fillForms: true },
  unsaved,
  encrypted: false,
  canUndo: unsaved,
  canRedo: false,
  recovery: "none",
});

async function openForm() {
  let emit: (event: OpenEvent) => void = () => {};
  const push = (event: OpenEvent) => act(() => emit(event));
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
  let doc = 9;
  let value = FIELD.value;
  // Like the main process: the new document is announced, then the command is answered.
  const editingApi = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>((_doc, edit) => {
      if (edit.kind === "setFieldValue") value = edit.value;
      doc += 1;
      push({ kind: "opened", tab: 1, info: info(doc, true) });
      return Promise.resolve();
    }),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
  const savingApi = {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
  } satisfies SavingApi;
  const formsApi = {
    getPageFields: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? [{ ...FIELD, value }] : [])),
  } satisfies FormsApi;
  const renderApi: RenderApi = { renderPage: () => new Promise(() => {}), cancel: () => Promise.resolve() };
  render(
    <App api={api} renderApi={renderApi} savingApi={savingApi} editingApi={editingApi} formsApi={formsApi} />,
  );
  push({ kind: "opening", tab: 1, displayName: "form.pdf" });
  push({ kind: "opened", tab: 1, info: info(9, false) });
  const canvas = screen.getByRole("main");
  Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
  Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 800 });
  act(() => canvas.dispatchEvent(new Event("scroll")));
  const user = userEvent.setup();
  const name = await screen.findByRole("textbox", { name: "Your name" });
  return { api, editingApi, savingApi, push, user, name };
}

const t = strings.saving;

describe("closing a tab while a field is being typed in (B2-09)", () => {
  it("sends the value first, and asks whether to save it (Ctrl+W)", async () => {
    const { api, editingApi, user, name } = await openForm();
    await user.type(name, "!");
    await user.keyboard("{Control>}w{/Control}");
    expect(await screen.findByRole("dialog", { name: t.askTitle })).toHaveTextContent(t.askOne("form.pdf"));
    expect(editingApi.applyEdit).toHaveBeenCalledWith(9, { kind: "setFieldValue", page: 0, field: 6, value: "Jane!" });
    expect(api.close).not.toHaveBeenCalled();
  });

  it("asks when the tab is closed with the mouse, which leaves the box", async () => {
    const { api, editingApi, user, name } = await openForm();
    await user.type(name, "!");
    await user.click(screen.getByRole("button", { name: strings.tabs.close("form.pdf") }));
    expect(await screen.findByRole("dialog", { name: t.askTitle })).toBeInTheDocument();
    expect(editingApi.applyEdit).toHaveBeenCalledTimes(1);
    expect(api.close).not.toHaveBeenCalled();
  });

  it("saves the document the value made, then closes the tab", async () => {
    const { api, savingApi, user, name } = await openForm();
    await user.type(name, "!");
    await user.keyboard("{Control>}w{/Control}");
    const dialog = await screen.findByRole("dialog", { name: t.askTitle });
    await user.click(within(dialog).getByRole("button", { name: t.save }));
    expect(savingApi.save).toHaveBeenCalledWith(10);
    await waitFor(() => expect(api.close).toHaveBeenCalledWith(1));
  });

  it("closes the tab at once when nothing was typed", async () => {
    const { api, editingApi, user, name } = await openForm();
    await user.click(name);
    await user.keyboard("{Control>}w{/Control}");
    expect(api.close).toHaveBeenCalledWith(1);
    expect(editingApi.applyEdit).not.toHaveBeenCalled();
  });
});

describe("closing the window while a field is being typed in (#153)", () => {
  it("sends the value first, and the main process then asks about the change it made", async () => {
    const { editingApi, savingApi, push, user, name } = await openForm();
    savingApi.closeWindow.mockImplementationOnce(() => {
      // As the main process does: the value made an unsaved change, so the window stays and the
      // page is asked again, naming the tab.
      push({ kind: "closeRequested", tabs: [1] });
      return Promise.reject({ code: "invalidArgument", message: "" });
    });
    await user.type(name, "!");
    // The window's close button: no tab is known to have unsaved changes yet.
    push({ kind: "closeRequested", tabs: [] });
    expect(await screen.findByRole("dialog", { name: t.askTitle })).toHaveTextContent(t.askOne("form.pdf"));
    expect(editingApi.applyEdit).toHaveBeenCalledWith(9, { kind: "setFieldValue", page: 0, field: 6, value: "Jane!" });
    expect(savingApi.closeWindow).toHaveBeenCalledTimes(1);
    expect(savingApi.closeWindow).toHaveBeenCalledWith(false);
  });

  it("saves the document the value made when the user says so, then closes the window", async () => {
    const { savingApi, push, user, name } = await openForm();
    savingApi.closeWindow.mockImplementationOnce(() => {
      push({ kind: "closeRequested", tabs: [1] });
      return Promise.reject({ code: "invalidArgument", message: "" });
    });
    await user.type(name, "!");
    push({ kind: "closeRequested", tabs: [] });
    const dialog = await screen.findByRole("dialog", { name: t.askTitle });
    await user.click(within(dialog).getByRole("button", { name: t.save }));
    await waitFor(() => expect(savingApi.save).toHaveBeenCalledWith(10));
    await waitFor(() => expect(savingApi.closeWindow).toHaveBeenCalledTimes(2));
  });

  it("closes the window at once when nothing was typed", async () => {
    const { editingApi, savingApi, push, user, name } = await openForm();
    await user.click(name);
    push({ kind: "closeRequested", tabs: [] });
    await waitFor(() => expect(savingApi.closeWindow).toHaveBeenCalledWith(false));
    expect(editingApi.applyEdit).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
