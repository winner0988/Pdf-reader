import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SavingApi } from "@/features/saving/api";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";

const t = strings.encryption;

function fakeSavingApi(encryptCopy: SavingApi["encryptCopy"]) {
  return {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
    encryptCopy: vi.fn<SavingApi["encryptCopy"]>(encryptCopy),
  } satisfies SavingApi;
}

function renderShell(api?: SavingApi, encrypted = false) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, encrypted } };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} savingApi={api} />);
  return { user };
}

type User = ReturnType<typeof userEvent.setup>;

async function menuItem(user: User) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  return screen.findByRole("menuitem", { name: new RegExp(strings.menu.encryptCopy) });
}

async function openDialog(user: User) {
  await user.click(await menuItem(user));
  return screen.findByRole("dialog", { name: t.title });
}

const field = (dialog: HTMLElement, label: string) => within(dialog).getByLabelText(label, { exact: true });
const start = (dialog: HTMLElement) => within(dialog).getByRole("button", { name: t.start });

async function typeInto(user: User, dialog: HTMLElement, label: string, text: string) {
  await user.type(field(dialog, label), text);
}

describe("encrypting a copy (B2-15)", () => {
  it("says what it does, and asks for nothing until something is asked for", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    for (const text of [t.description, t.note, t.signatures, t.openPasswordHint, t.permissionsPasswordHint]) {
      expect(dialog).toHaveTextContent(text);
    }
    expect(start(dialog)).toBeDisabled();
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.problem.nothing);
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.encryptCopy).not.toHaveBeenCalled();
  });

  it("sends the open password alone, then says it is done", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await typeInto(user, dialog, t.openPassword, "secret");
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.problem.openMismatch);
    await typeInto(user, dialog, `${t.openPassword}（${t.again}）`, "secret");
    expect(within(dialog).getByRole("status")).toHaveTextContent("");
    await user.click(start(dialog));

    expect(api.encryptCopy).toHaveBeenCalledExactlyOnceWith({
      doc: 5,
      openPassword: "secret",
      permissionsPassword: null,
      restrictions: { print: false, copy: false, modify: false },
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.done);
    // It is a copy: the document itself is not saved.
    expect(api.save).not.toHaveBeenCalled();
  });

  it("needs a permissions password, different from the open one, for a restriction", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("checkbox", { name: t.restrict.print }));
    await user.click(within(dialog).getByRole("checkbox", { name: t.restrict.modify }));
    expect(start(dialog)).toBeDisabled();
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.problem.permissionsNeeded);

    await typeInto(user, dialog, t.permissionsPassword, "owner");
    await typeInto(user, dialog, `${t.permissionsPassword}（${t.again}）`, "owner");
    expect(start(dialog)).toBeEnabled();
    // The same password for both would let whoever opens the copy lift the restrictions.
    await typeInto(user, dialog, t.openPassword, "owner");
    await typeInto(user, dialog, `${t.openPassword}（${t.again}）`, "owner");
    expect(start(dialog)).toBeDisabled();
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.problem.same);
    await user.clear(field(dialog, t.openPassword));
    await user.clear(field(dialog, `${t.openPassword}（${t.again}）`));
    await user.click(start(dialog));

    expect(api.encryptCopy).toHaveBeenCalledExactlyOnceWith({
      doc: 5,
      openPassword: null,
      permissionsPassword: "owner",
      restrictions: { print: true, copy: false, modify: true },
    });
  });

  it("says a password is too long, in bytes", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    const long = "密".repeat(43);
    await typeInto(user, dialog, t.openPassword, long);
    await typeInto(user, dialog, `${t.openPassword}（${t.again}）`, long);
    expect(start(dialog)).toBeDisabled();
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.problem.tooLong(127));
  });

  it("stays open when the system's dialog is closed, and explains a failure", async () => {
    let answer: () => Promise<boolean> = () => Promise.resolve(false);
    const api = fakeSavingApi(() => answer());
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await typeInto(user, dialog, t.openPassword, "secret");
    await typeInto(user, dialog, `${t.openPassword}（${t.again}）`, "secret");

    await user.click(start(dialog));
    await waitFor(() => expect(start(dialog)).toBeEnabled());
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();

    answer = () => Promise.reject({ code: "invalidArgument", message: "" });
    await user.click(start(dialog));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failed);
  });

  it("keeps no password once it is closed", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    let dialog = await openDialog(user);
    await typeInto(user, dialog, t.openPassword, "secret");
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));

    dialog = await openDialog(user);
    expect(field(dialog, t.openPassword)).toHaveValue("");
    expect(field(dialog, `${t.openPassword}（${t.again}）`)).toHaveValue("");
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
