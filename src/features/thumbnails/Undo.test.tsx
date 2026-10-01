import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellDocument, ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";

function fakeEditingApi() {
  return {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
  } satisfies EditingApi;
}

function renderShell(document: Partial<ShellDocument>) {
  const api = fakeEditingApi();
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, ...document } };
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} editingApi={api} />);
  return { api, user: userEvent.setup() };
}

async function menuItem(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  return screen.findByRole("menuitem", { name: new RegExp(`^${name}`) });
}

describe("undo and redo (B2-05)", () => {
  it("undoes with Ctrl+Z and redoes with Ctrl+Y or Ctrl+Shift+Z", async () => {
    const { api, user } = renderShell({ unsaved: true, canUndo: true, canRedo: true });
    await user.keyboard("{Control>}z{/Control}");
    expect(api.undo).toHaveBeenCalledWith(5, undefined);
    await user.keyboard("{Control>}y{/Control}");
    await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}");
    expect(api.redo).toHaveBeenCalledTimes(2);
  });

  it("offers them in the menu only when there is something to undo or redo", async () => {
    const { api, user } = renderShell({ unsaved: true, canUndo: true, canRedo: false });
    await user.click(await menuItem(user, strings.menu.undo));
    expect(api.undo).toHaveBeenCalledWith(5, undefined);
    expect(await menuItem(user, strings.menu.redo)).toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Escape}");
    await user.keyboard("{Control>}y{/Control}");
    expect(api.redo).not.toHaveBeenCalled();
  });

  it("leaves Ctrl+Z to a text field", async () => {
    const { api, user } = renderShell({ unsaved: true, canUndo: true });
    await user.keyboard("{Control>}f{/Control}");
    await user.keyboard("{Control>}z{/Control}");
    expect(api.undo).not.toHaveBeenCalled();
  });

  it("asks again for the password of a document opened with one, and says when it is wrong (#94)", async () => {
    const { api, user } = renderShell({ unsaved: true, canUndo: true });
    const t = strings.pages.undoPassword;
    // The main process wants the password; a wrong one is refused the same way.
    api.undo.mockImplementation((_doc, password) =>
      password === "user" ? Promise.resolve() : Promise.reject({ code: "encrypted", message: "" }),
    );
    await user.keyboard("{Control>}z{/Control}");
    const dialog = await screen.findByRole("dialog", { name: t.title });
    expect(api.undo).toHaveBeenCalledWith(5, undefined);
    expect(within(dialog).queryByRole("alert")).toBeNull();

    const field = within(dialog).getByLabelText(t.label);
    await user.type(field, "wrong{Enter}");
    expect(api.undo).toHaveBeenLastCalledWith(5, "wrong");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.wrong);
    // The field never keeps a password once it is sent.
    expect(field).toHaveValue("");

    await user.type(field, "user{Enter}");
    expect(api.undo).toHaveBeenLastCalledWith(5, "user");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.title })).toBeNull());
  });

  it("does nothing when the password is not given", async () => {
    const { api, user } = renderShell({ unsaved: true, canUndo: true });
    api.undo.mockRejectedValue({ code: "encrypted", message: "" });
    await user.keyboard("{Control>}z{/Control}");
    const dialog = await screen.findByRole("dialog", { name: strings.pages.undoPassword.title });
    await user.click(within(dialog).getByRole("button", { name: strings.pages.undoPassword.cancel }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: strings.pages.undoPassword.title })).toBeNull());
    expect(api.undo).toHaveBeenCalledTimes(1);
  });
});
