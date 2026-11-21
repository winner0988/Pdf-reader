import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SavingApi } from "@/features/saving/api";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import { expectAccessible } from "@/test/axe";

const t = strings.decryption;

function fakeSavingApi(decryptCopy: SavingApi["decryptCopy"]) {
  return {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false, redaction: null })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
    encryptCopy: vi.fn<SavingApi["encryptCopy"]>(() => Promise.resolve(true)),
    decryptCopy: vi.fn<SavingApi["decryptCopy"]>(decryptCopy),
    pageMarksCopy: vi.fn<SavingApi["pageMarksCopy"]>(() => Promise.resolve(true)),
    compressImages: vi.fn<SavingApi["compressImages"]>(() => Promise.resolve(null)),
  } satisfies SavingApi;
}

function renderShell(api?: SavingApi, encrypted = true) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, encrypted } };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} savingApi={api} />);
  return { user };
}

type User = ReturnType<typeof userEvent.setup>;

async function menuItem(user: User) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  return screen.findByRole("menuitem", { name: new RegExp(strings.menu.decryptCopy) });
}

async function openDialog(user: User) {
  await user.click(await menuItem(user));
  return screen.findByRole("dialog", { name: t.title });
}

const field = (dialog: HTMLElement) => within(dialog).getByLabelText(t.password, { exact: true });
const start = (dialog: HTMLElement) => within(dialog).getByRole("button", { name: t.start });

describe("saving a copy without its password (docs/architecture/decrypt-copy.md)", () => {
  it("says what it does, and has submit disabled until a password is typed", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    expect(dialog).toHaveTextContent(t.description);
    expect(start(dialog)).toBeDisabled();
    await expectAccessible(dialog);

    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.decryptCopy).not.toHaveBeenCalled();
  });

  it("sends the password, then says it is done", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    await user.type(field(dialog), "secret");
    expect(start(dialog)).toBeEnabled();
    await user.click(start(dialog));

    expect(api.decryptCopy).toHaveBeenCalledExactlyOnceWith({
      doc: 5,
      password: "secret",
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.done);
    expect(api.save).not.toHaveBeenCalled();
  });

  it("keeps no password once it is closed", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api);
    let dialog = await openDialog(user);

    await user.type(field(dialog), "secret");
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));

    dialog = await openDialog(user);
    expect(field(dialog)).toHaveValue("");
  });

  it("shows wrong password error on encrypted error and clears password", async () => {
    const api = fakeSavingApi(() => Promise.reject({ code: "encrypted", message: "" }));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    await user.type(field(dialog), "wrong-password");
    await user.click(start(dialog));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.wrongPassword);
    expect(field(dialog)).toHaveValue("");
  });

  it("shows permissions password needed error on notAllowed error and clears password", async () => {
    const api = fakeSavingApi(() => Promise.reject({ code: "notAllowed", message: "" }));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    await user.type(field(dialog), "user-password");
    await user.click(start(dialog));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.notAllowed);
    expect(field(dialog)).toHaveValue("");
  });

  it("shows generic failure message on unexpected error", async () => {
    const api = fakeSavingApi(() => Promise.reject({ code: "internal", message: "" }));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    await user.type(field(dialog), "some-password");
    await user.click(start(dialog));

    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failed);
  });

  it("stays open when the system save dialog is cancelled", async () => {
    const api = fakeSavingApi(() => Promise.resolve(false));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    await user.type(field(dialog), "secret");
    await user.click(start(dialog));

    await waitFor(() => expect(start(dialog)).toBeDisabled());
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();
    expect(field(dialog)).toHaveValue("");
  });

  it("is not offered for an unencrypted document, and says why", async () => {
    const api = fakeSavingApi(() => Promise.resolve(true));
    const { user } = renderShell(api, false);
    const item = await menuItem(user);

    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(t.unencryptedBlocked);
  });

  it("needs the main process (not demo data)", async () => {
    const { user } = renderShell(undefined, true);
    expect(await menuItem(user)).toHaveAttribute("aria-disabled", "true");
  });
});
