import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SavingApi } from "@/features/saving/api";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { SaveResult } from "@/ipc/generated/contract";

const t = strings.saving;

function fakeSavingApi(overrides: Partial<SavingApi> = {}) {
  return {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve({ incremental: false })),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
    encryptCopy: vi.fn<SavingApi["encryptCopy"]>(() => Promise.resolve(true)),
    ...overrides,
  } satisfies SavingApi;
}

function renderShell(api: SavingApi, unsaved: boolean) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, unsaved } };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} savingApi={api} />);
  return { user };
}

async function menuItem(user: ReturnType<typeof userEvent.setup>, name: string) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  // From the start: "加密並另存新檔…" (B2-15) ends as "另存新檔…" does.
  return screen.findByRole("menuitem", { name: new RegExp(`^${name}`) });
}

const statusText = () => screen.getByRole("contentinfo").textContent ?? "";

describe("saving (B2-02)", () => {
  it("offers saving only with unsaved changes; Ctrl+S saves and says so", async () => {
    const api = fakeSavingApi();
    const { user } = renderShell(api, true);

    expect(await menuItem(user, strings.menu.save)).not.toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Escape}");
    await user.keyboard("{Control>}s{/Control}");
    expect(api.save).toHaveBeenCalledWith(5);
    await waitFor(() => expect(statusText()).toContain(t.saved));
  });

  it("has nothing to save without changes", async () => {
    const api = fakeSavingApi();
    const { user } = renderShell(api, false);

    expect(await menuItem(user, strings.menu.save)).toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Escape}");
    await user.keyboard("{Control>}s{/Control}");
    expect(api.save).not.toHaveBeenCalled();
  });

  it("says when a signed document's changes were appended", async () => {
    const api = fakeSavingApi({ save: vi.fn(() => Promise.resolve({ incremental: true })) });
    const { user } = renderShell(api, true);

    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(statusText()).toContain(t.savedIncremental));
  });

  it("saves as another file from the menu and Ctrl+Shift+S; a closed dialog changes nothing", async () => {
    let answer: (result: SaveResult | null) => void = () => {};
    const api = fakeSavingApi({
      saveAs: vi.fn(() => new Promise<SaveResult | null>((resolve) => (answer = resolve))),
    });
    const { user } = renderShell(api, false);

    await user.click(await menuItem(user, strings.menu.saveAs));
    expect(api.saveAs).toHaveBeenCalledWith(5);
    await act(() => answer(null));
    expect(statusText()).not.toContain(t.saved);

    await user.keyboard("{Control>}{Shift>}s{/Shift}{/Control}");
    expect(api.saveAs).toHaveBeenCalledTimes(2);
    await act(() => answer({ incremental: false }));
    expect(statusText()).toContain(t.saved);
  });

  it("explains a failed save, keeps the changes and offers saving a copy", async () => {
    const api = fakeSavingApi({
      save: vi.fn(() => Promise.reject({ code: "changedOnDisk", message: "" })),
    });
    const { user } = renderShell(api, true);

    await user.keyboard("{Control>}s{/Control}");
    const dialog = await screen.findByRole("dialog", { name: t.failedTitle });
    expect(dialog).toHaveTextContent(strings.error.messages.changedOnDisk);
    expect(dialog).toHaveTextContent(t.keptChanges);
    await user.click(within(dialog).getByRole("button", { name: t.saveAs }));
    expect(api.saveAs).toHaveBeenCalledWith(5);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.failedTitle })).not.toBeInTheDocument());
  });

  it("never lets Ctrl+S save the page itself, even with no document", () => {
    render(<ReaderShell state={{ kind: "empty" }} onOpen={vi.fn()} loadingDelayMs={0} />);
    const handled = !fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    expect(handled).toBe(true);
  });
});
