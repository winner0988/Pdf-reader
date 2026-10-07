import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SavingApi } from "@/features/saving/api";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";

const t = strings.privacyExport;

function fakeSavingApi(privacyExport: SavingApi["privacyExport"]) {
  return {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(privacyExport),
    encryptCopy: vi.fn<SavingApi["encryptCopy"]>(() => Promise.resolve(true)),
  } satisfies SavingApi;
}

function renderShell(api?: SavingApi, encrypted = false) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, encrypted } };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} savingApi={api} />);
  return { user };
}

async function menuItem(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  return screen.findByRole("menuitem", { name: new RegExp(strings.menu.privacyExport) });
}

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await menuItem(user));
  return screen.findByRole("dialog", { name: t.title });
}

describe("privacy export (B2-03)", () => {
  it("says what the copy loses and keeps before anything is written", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    for (const item of [...t.removed, ...t.kept, t.signatures, t.description]) {
      expect(dialog).toHaveTextContent(item);
    }
    expect(api.privacyExport).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.privacyExport).not.toHaveBeenCalled();
  });

  it("asks the main process for the copy, then says it is done", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("button", { name: t.start }));

    expect(api.privacyExport).toHaveBeenCalledExactlyOnceWith(5);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.done);
    // It is a copy: the document itself is not saved.
    expect(api.save).not.toHaveBeenCalled();
    expect(api.saveAs).not.toHaveBeenCalled();
  });

  it("stays open when the system's dialog is closed, and explains a failure", async () => {
    let answer: () => Promise<boolean> = () => Promise.resolve(false);
    const api = fakeSavingApi(() => answer());
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();

    answer = () => Promise.reject({ code: "unwritable", message: "" });
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failed);
  });

  it("is not offered for an encrypted document, and says why", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api, true);
    const item = await menuItem(user);
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(t.encrypted);
  });

  it("needs the main process (not demo data)", async () => {
    const { user } = renderShell(undefined);
    expect(await menuItem(user)).toHaveAttribute("aria-disabled", "true");
  });
});
