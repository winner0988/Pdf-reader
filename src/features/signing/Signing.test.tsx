// Signing a document (B3-06, ADR 0020): the menu item, what the dialog says of a document that
// cannot be signed, and what it sends to sign one: the certificate file's number, the password once,
// where the signature shows and why. The page never gets a path.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { SigningApi } from "@/features/signing/api";
import { argsOf, DEFAULT_FORM, failure, reasonFits } from "@/features/signing/signing";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type SignCheck, type SignResult } from "@/ipc/generated/contract";

const t = strings.signing;
const PASSWORD = "correct horse";

function fakeApi(check: SignCheck = "ready", sign?: SigningApi["sign"]) {
  return {
    check: vi.fn<SigningApi["check"]>(() => Promise.resolve(check)),
    chooseFile: vi.fn<SigningApi["chooseFile"]>(() => Promise.resolve({ file: 3, name: "我的憑證.pfx" })),
    sign: vi.fn<SigningApi["sign"]>(sign ?? (() => Promise.resolve({ name: "報告（已簽署）.pdf" }))),
  } satisfies SigningApi;
}

function renderShell(api?: SigningApi, encrypted = false) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, encrypted } };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} signingApi={api} />);
  return { user };
}

async function menuItem(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  return screen.findByRole("menuitem", { name: new RegExp(strings.menu.sign) });
}

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await menuItem(user));
  return screen.findByRole("dialog", { name: t.title });
}

/** Chooses the certificate file and types its password. */
async function fillIn(user: ReturnType<typeof userEvent.setup>, dialog: HTMLElement) {
  await user.click(await within(dialog).findByRole("button", { name: t.chooseFile }));
  await within(dialog).findByText("我的憑證.pfx");
  await user.type(within(dialog).getByLabelText(t.passwordLabel), PASSWORD);
}

describe("what is sent to sign (B3-06)", () => {
  it("sends the page shown as an index, a reason only if there is one, and this computer's time zone", () => {
    const now = new Date();
    expect(argsOf(5, 3, PASSWORD, DEFAULT_FORM, 2, now)).toEqual({
      doc: 5,
      file: 3,
      password: PASSWORD,
      place: { pageIndex: 1, corner: "bottomRight" },
      reason: null,
      utcOffsetMinutes: -now.getTimezoneOffset(),
    });
    expect(argsOf(5, 3, PASSWORD, { corner: null, reason: "  同意  " }, 2, now)).toMatchObject({
      place: null,
      reason: "同意",
    });
    expect(argsOf(5, 3, PASSWORD, { corner: "topLeft", reason: "   " }, 1, now)).toMatchObject({
      place: { pageIndex: 0, corner: "topLeft" },
      reason: null,
    });
  });

  it("takes a reason as long as the main process does, counted in UTF-8", () => {
    expect(reasonFits("x".repeat(LIMITS.maxSigningReasonBytes))).toBe(true);
    expect(reasonFits("x".repeat(LIMITS.maxSigningReasonBytes + 1))).toBe(false);
    // Three bytes each.
    expect(reasonFits("同".repeat(Math.floor(LIMITS.maxSigningReasonBytes / 3)))).toBe(true);
    expect(reasonFits("同".repeat(Math.floor(LIMITS.maxSigningReasonBytes / 3) + 1))).toBe(false);
  });

  it("says why it did not work without telling a wrong password from a file that is no certificate", () => {
    expect(failure("certificateUnreadable")).toBe(strings.error.messages.certificateUnreadable);
    expect(failure("certificateExpired")).toBe(strings.error.messages.certificateExpired);
    expect(failure("certificateNotForSigning")).toBe(strings.error.messages.certificateNotForSigning);
    expect(failure("signatureNotValid")).toBe(strings.error.messages.signatureNotValid);
    expect(failure("notAllowed")).toBe(t.certified);
    expect(failure("workerCrashed")).toBe(t.failed);
  });
});

describe("the signing dialog (B3-06)", () => {
  it("is not offered without the main process, and says so for an encrypted document", async () => {
    const { user } = renderShell(undefined);
    expect(await menuItem(user)).toHaveAttribute("aria-disabled", "true");
  });

  it("says an encrypted document is not signed", async () => {
    const { user } = renderShell(fakeApi(), true);
    const item = await menuItem(user);
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(t.encryptedShort);
  });

  it.each([
    ["unsaved", t.unsaved],
    ["certified", t.certified],
    ["encrypted", t.encrypted],
  ] as const)("says why a %s document cannot be signed, and asks nothing", async (check, message) => {
    const api = fakeApi(check);
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(message);
    expect(within(dialog).queryByLabelText(t.passwordLabel)).toBeNull();
    expect(within(dialog).queryByRole("button", { name: t.start })).toBeNull();
    expect(api.chooseFile).not.toHaveBeenCalled();
  });

  it("signs with the file chosen and the password typed, and says where the copy went", async () => {
    const api = fakeApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    const start = await within(dialog).findByRole("button", { name: t.start });
    // Nothing to sign with yet.
    expect(start).toBeDisabled();
    await fillIn(user, dialog);
    expect(start).toBeEnabled();
    await user.type(within(dialog).getByLabelText(t.reasonLabel), "同意本合約");
    await user.click(start);

    expect(api.sign).toHaveBeenCalledTimes(1);
    const args = api.sign.mock.calls[0][0];
    expect(args).toMatchObject({
      doc: 5,
      file: 3,
      password: PASSWORD,
      place: { pageIndex: 0, corner: "bottomRight" },
      reason: "同意本合約",
    });
    expect(Object.keys(args).sort()).toEqual(["doc", "file", "password", "place", "reason", "utcOffsetMinutes"]);
    expect(await within(dialog).findByRole("status")).toHaveTextContent(t.result("報告（已簽署）.pdf"));
    expect(screen.getByRole("dialog", { name: t.resultTitle })).toBeInTheDocument();
  });

  it("signs without a box, or with it in another corner", async () => {
    const api = fakeApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await fillIn(user, dialog);
    await user.selectOptions(within(dialog).getByLabelText(t.cornerLabel), t.corners.topLeft);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(api.sign.mock.calls[0][0].place).toEqual({ pageIndex: 0, corner: "topLeft" });
  });

  it("can sign so that nothing shows on the page", async () => {
    const api = fakeApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await fillIn(user, dialog);
    await user.click(within(dialog).getByRole("radio", { name: t.invisible }));
    expect(within(dialog).queryByLabelText(t.cornerLabel)).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(api.sign.mock.calls[0][0].place).toBeNull();
  });

  it("forgets the password when signing fails, and never shows it", async () => {
    const api = fakeApi("ready", () => Promise.reject({ code: "certificateUnreadable", message: "" }));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await fillIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      strings.error.messages.certificateUnreadable,
    );
    expect(within(dialog).getByLabelText(t.passwordLabel)).toHaveValue("");
    expect(within(dialog).getByLabelText(t.passwordLabel)).toHaveAttribute("aria-invalid", "true");
    expect(document.body.textContent).not.toContain(PASSWORD);
    // The file chosen stays: only the password is typed again.
    expect(within(dialog).getByText("我的憑證.pfx")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: t.start })).toBeDisabled();
  });

  it("stays as it was when the user closes the save dialog", async () => {
    const api = fakeApi("ready", () => Promise.resolve(null as SignResult | null));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await fillIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());
    expect(within(dialog).queryByRole("alert")).toBeNull();
    expect(within(dialog).getByLabelText(t.passwordLabel)).toHaveValue(PASSWORD);
  });

  it("does not take a reason longer than the main process does", async () => {
    const api = fakeApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await fillIn(user, dialog);
    const reason = within(dialog).getByLabelText(t.reasonLabel);
    await user.click(reason);
    await user.paste("x".repeat(LIMITS.maxSigningReasonBytes + 1));
    expect(within(dialog).getByText(t.reasonTooLong)).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: t.start })).toBeDisabled();
  });

  it("starts over every time it opens, without the password or the file", async () => {
    const api = fakeApi();
    const { user } = renderShell(api);
    let dialog = await openDialog(user);
    await fillIn(user, dialog);
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    dialog = await openDialog(user);
    expect(await within(dialog).findByLabelText(t.passwordLabel)).toHaveValue("");
    expect(within(dialog).getByText(t.noFile)).toBeInTheDocument();
    expect(api.check).toHaveBeenCalledTimes(2);
  });
});
