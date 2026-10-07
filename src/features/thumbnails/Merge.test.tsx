import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellDocument, ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";
import type { PagesSource } from "@/ipc/generated/contract";

const t = strings.pages;
const taken: PagesSource = { source: 7, pages: 3 };

function fakeEditingApi() {
  return {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
    pickPagesSource: vi.fn<NonNullable<EditingApi["pickPagesSource"]>>(() => Promise.resolve(taken)),
    unlockPagesSource: vi.fn<NonNullable<EditingApi["unlockPagesSource"]>>(() => Promise.resolve(taken)),
  } satisfies EditingApi;
}

function renderShell(document: Partial<ShellDocument> = {}) {
  const api = fakeEditingApi();
  const state = (overrides: Partial<ShellDocument>): ShellState => ({
    kind: "open",
    document: { ...demoDocument, doc: 5, session: 1, ...document, ...overrides },
  });
  const view = render(<ReaderShell state={state({})} onOpen={vi.fn()} loadingDelayMs={0} editingApi={api} />);
  const rerender = (overrides: Partial<ShellDocument>) =>
    view.rerender(<ReaderShell state={state(overrides)} onOpen={vi.fn()} loadingDelayMs={0} editingApi={api} />);
  return { api, rerender, user: userEvent.setup() };
}

async function takeAfterPage(user: ReturnType<typeof userEvent.setup>, number: number) {
  await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));
  fireEvent.contextMenu(await screen.findByRole("option", { name: strings.canvas.page(number) }));
  await user.click(await screen.findByRole("menuitem", { name: new RegExp(`^${t.insertFileAfter}`) }));
}

describe("the pages of another file (B2-06)", () => {
  beforeEach(() => {
    // jsdom has no canvas.
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  });

  it("are asked for in the main process and put in where the menu says", async () => {
    const { api, user } = renderShell();
    await takeAfterPage(user, 2);
    expect(api.pickPagesSource).toHaveBeenCalledWith(5);
    await waitFor(() =>
      expect(api.applyEdit).toHaveBeenCalledWith(5, { kind: "insertPages", at: 2, source: 7 }),
    );
    expect(api.unlockPagesSource).not.toHaveBeenCalled();
  });

  it("are not put in when the dialog is closed", async () => {
    const { api, user } = renderShell();
    api.pickPagesSource.mockResolvedValue(null);
    await takeAfterPage(user, 2);
    await waitFor(() => expect(api.pickPagesSource).toHaveBeenCalled());
    expect(api.applyEdit).not.toHaveBeenCalled();
  });

  it("ask for the password of an encrypted file, until it is right, and are put in then", async () => {
    const { api, user } = renderShell();
    api.pickPagesSource.mockRejectedValue({ code: "encrypted", message: "" });
    api.unlockPagesSource.mockImplementation((_doc, password) =>
      password === "user" ? Promise.resolve(taken) : Promise.reject({ code: "encrypted", message: "" }),
    );
    await takeAfterPage(user, 3);
    const dialog = await screen.findByRole("dialog", { name: t.sourcePassword.title });
    expect(within(dialog).queryByRole("alert")).toBeNull();
    const field = within(dialog).getByLabelText(t.sourcePassword.label);
    await user.type(field, "wrong{Enter}");
    expect(api.unlockPagesSource).toHaveBeenLastCalledWith(5, "wrong");
    // Said, and the dialog stays for another try; the field never keeps what was sent.
    expect(await within(await screen.findByRole("dialog", { name: t.sourcePassword.title })).findByRole("alert")).toHaveTextContent(
      t.sourcePassword.wrong,
    );
    expect(api.applyEdit).not.toHaveBeenCalled();
    await user.type(screen.getByLabelText(t.sourcePassword.label), "user{Enter}");
    expect(api.unlockPagesSource).toHaveBeenLastCalledWith(5, "user");
    await waitFor(() =>
      expect(api.applyEdit).toHaveBeenCalledWith(5, { kind: "insertPages", at: 3, source: 7 }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.sourcePassword.title })).toBeNull());
  });

  it("are not put in when the password is not given", async () => {
    const { api, user } = renderShell();
    api.pickPagesSource.mockRejectedValue({ code: "encrypted", message: "" });
    await takeAfterPage(user, 3);
    const dialog = await screen.findByRole("dialog", { name: t.sourcePassword.title });
    await user.click(within(dialog).getByRole("button", { name: t.sourcePassword.cancel }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.sourcePassword.title })).toBeNull());
    expect(api.unlockPagesSource).not.toHaveBeenCalled();
    expect(api.applyEdit).not.toHaveBeenCalled();
    // Nothing is said: the user chose not to.
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("say why an encrypted file that the author forbids cannot be used", async () => {
    const { api, user } = renderShell();
    api.pickPagesSource.mockRejectedValue({ code: "encrypted", message: "" });
    api.unlockPagesSource.mockRejectedValue({ code: "notAllowed", message: "" });
    await takeAfterPage(user, 3);
    const dialog = await screen.findByRole("dialog", { name: t.sourcePassword.title });
    await user.type(within(dialog).getByLabelText(t.sourcePassword.label), "user{Enter}");
    expect(await screen.findByRole("alert")).toHaveTextContent(t.sourceNotAllowed);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.sourcePassword.title })).toBeNull());
    expect(api.applyEdit).not.toHaveBeenCalled();
  });
});

describe("what the pages of another file brought to the banner (B2-06)", () => {
  it("is shown again once it says something it did not when it was closed, and not for what it said", async () => {
    const own = { findings: [{ kind: "javaScript" as const, count: 1 }] };
    const { rerender, user } = renderShell({ findings: own.findings, scanComplete: true });
    const banner = () => screen.queryByRole("region", { name: strings.banner.label });
    expect(banner()).not.toBeNull();
    await user.click(within(banner()!).getByRole("button", { name: strings.banner.dismiss }));
    expect(banner()).toBeNull();
    // The same words: still closed (an edit gives the document a new id, nothing else changes).
    rerender({ doc: 6, findings: own.findings, scanComplete: true, unsaved: true });
    expect(banner()).toBeNull();
    // More to say: the pages of a file that had a launch action came in.
    const more = [...own.findings, { kind: "launch" as const, count: 1 }];
    rerender({ doc: 7, findings: more, scanComplete: true, unsaved: true });
    expect(banner()).not.toBeNull();
    // Undone, it says what it said when it was closed: it stays closed.
    rerender({ doc: 8, findings: own.findings, scanComplete: true, unsaved: true });
    expect(banner()).toBeNull();
  });
});
