import { render, screen, waitFor } from "@testing-library/react";
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
    expect(api.undo).toHaveBeenCalledWith(5);
    await user.keyboard("{Control>}y{/Control}");
    await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}");
    expect(api.redo).toHaveBeenCalledTimes(2);
  });

  it("offers them in the menu only when there is something to undo or redo", async () => {
    const { api, user } = renderShell({ unsaved: true, canUndo: true, canRedo: false });
    await user.click(await menuItem(user, strings.menu.undo));
    expect(api.undo).toHaveBeenCalledWith(5);
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

  it("says why a document opened with a password has no undo", async () => {
    const { api, user } = renderShell({ unsaved: true, canUndo: false });
    await user.keyboard("{Control>}z{/Control}");
    expect(api.undo).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByRole("contentinfo")).toHaveTextContent(strings.pages.noUndo));
  });
});
