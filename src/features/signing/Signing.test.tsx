// Signing a document (B3-06, ADR 0020): the menu item, what the dialog says of a document that
// cannot be signed, and what it sends to sign one: the certificate file's number, the password once,
// where the signature shows and why. The page never gets a path.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SignaturesApi } from "@/features/annotations/signaturesApi";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { SigningApi } from "@/features/signing/api";
import { argsOf, dateOf, DEFAULT_FORM, failure, reasonFits, usable } from "@/features/signing/signing";
import { strings } from "@/i18n";
import {
  LIMITS,
  type SignCheck,
  type SignatureImage,
  type SignResult,
  type StoreCertificate,
} from "@/ipc/generated/contract";

const t = strings.signing;
const PASSWORD = "correct horse";

/** A natural person's certificate on a card, and an older one that has expired. */
const CARD: StoreCertificate = {
  certificate: 7,
  subject: "王小明",
  issuer: "內政部憑證管理中心",
  expires: { year: 2030, month: 5, day: 14 },
  problem: null,
};
const EXPIRED: StoreCertificate = {
  certificate: 8,
  subject: "王小明（舊卡）",
  issuer: "內政部憑證管理中心",
  expires: { year: 2024, month: 1, day: 2 },
  problem: "expired",
};

/** The main process, with no certificate on this computer unless `listed` says so. */
function fakeApi(check: SignCheck = "ready", sign?: SigningApi["sign"], listed: StoreCertificate[] = []) {
  return {
    check: vi.fn<SigningApi["check"]>(() => Promise.resolve(check)),
    chooseFile: vi.fn<SigningApi["chooseFile"]>(() => Promise.resolve({ file: 3, name: "我的憑證.pfx" })),
    listCertificates: vi.fn<SigningApi["listCertificates"]>(() => Promise.resolve(listed)),
    sign: vi.fn<SigningApi["sign"]>(sign ?? (() => Promise.resolve({ name: "報告（已簽署）.pdf" }))),
  } satisfies SigningApi;
}

function fakeSignaturesApi(initial: SignatureImage[] = []) {
  let list = [...initial];
  let nextId = (initial.reduce((max, sig) => Math.max(max, sig.id), 0) || 0) + 1;
  return {
    getSignatureImages: vi.fn<SignaturesApi["getSignatureImages"]>(() => Promise.resolve(list)),
    getSignatureImagePng: vi.fn<SignaturesApi["getSignatureImagePng"]>(() => Promise.resolve([137, 80, 78, 71])),
    addSignatureImage: vi.fn<SignaturesApi["addSignatureImage"]>(() => {
      list = [...list, { id: nextId++, width: 120, height: 60 }];
      return Promise.resolve(list);
    }),
    importSignatureImage: vi.fn<SignaturesApi["importSignatureImage"]>(() => {
      list = [...list, { id: nextId++, width: 160, height: 80 }];
      return Promise.resolve(list);
    }),
    removeSignatureImage: vi.fn<SignaturesApi["removeSignatureImage"]>(() => Promise.resolve(list)),
    removeAllSignatureImages: vi.fn<SignaturesApi["removeAllSignatureImages"]>(() => Promise.resolve([])),
    prepareSignatureStamp: vi.fn<SignaturesApi["prepareSignatureStamp"]>(() =>
      Promise.resolve({ image: 1, width: 120, height: 60 }),
    ),
  } satisfies SignaturesApi;
}

function renderShell(api?: SigningApi, encrypted = false, signaturesApi?: SignaturesApi) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, encrypted } };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} signingApi={api} signaturesApi={signaturesApi} />);
  return { user };
}

/** What the dialog sent to sign, the first time. */
function sent(api: ReturnType<typeof fakeApi>) {
  const call = api.sign.mock.calls[0];
  if (!call) throw new Error("nothing was signed");
  return call[0];
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
    const file = { kind: "file", file: 3, password: PASSWORD } as const;
    expect(argsOf(5, file, DEFAULT_FORM, 2, now)).toEqual({
      doc: 5,
      key: file,
      place: { pageIndex: 1, corner: "bottomRight" },
      reason: null,
      utcOffsetMinutes: -now.getTimezoneOffset(),
    });
    expect(
      argsOf(5, file, { corner: null, reason: "  同意  ", withPicture: true, signatureImage: 3 }, 2, now),
    ).toMatchObject({
      place: null,
      reason: "同意",
    });
    expect(
      argsOf(
        5,
        { kind: "store", certificate: 7 },
        { corner: "topLeft", reason: "   ", withPicture: false, signatureImage: null },
        1,
        now,
      ),
    ).toMatchObject({
      key: { kind: "store", certificate: 7 },
      place: { pageIndex: 0, corner: "topLeft" },
      reason: null,
    });
    expect(
      argsOf(
        5,
        file,
        { corner: "bottomLeft", reason: "", withPicture: true, signatureImage: 4 },
        1,
        now,
      ),
    ).toMatchObject({
      place: { pageIndex: 0, corner: "bottomLeft", signatureImage: 4 },
    });
  });

  it("offers the certificates that can sign, and writes their day as dates are written here", () => {
    expect(usable([EXPIRED, CARD])).toEqual([CARD]);
    expect(dateOf(CARD.expires)).toBe("2030-05-14");
    expect(dateOf({ year: 2036, month: 12, day: 31 })).toBe("2036-12-31");
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
    expect(failure("certificateUnavailable")).toBe(strings.error.messages.certificateUnavailable);
    expect(failure("signatureImageUnavailable")).toBe(strings.error.messages.signatureImageUnavailable);
    expect(failure("notAllowed")).toBe(t.certified);
    expect(failure("workerCrashed")).toBe(t.failed);
    // The user closed the window that asked for the PIN: nothing to say.
    expect(failure("cancelled")).toBeNull();
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
    const args = sent(api);
    expect(args).toMatchObject({
      doc: 5,
      key: { kind: "file", file: 3, password: PASSWORD },
      place: { pageIndex: 0, corner: "bottomRight" },
      reason: "同意本合約",
    });
    expect(Object.keys(args).sort()).toEqual(["doc", "key", "place", "reason", "utcOffsetMinutes"]);
    expect(await within(dialog).findByRole("status")).toHaveTextContent(t.result("報告（已簽署）.pdf"));
    expect(screen.getByRole("dialog", { name: t.resultTitle })).toBeInTheDocument();
    // Its own name: the dialog's corner button is the one called "關閉".
    await user.click(within(dialog).getByRole("button", { name: t.done }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("signs without a box, or with it in another corner", async () => {
    const api = fakeApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await fillIn(user, dialog);
    await user.selectOptions(within(dialog).getByLabelText(t.cornerLabel), t.corners.topLeft);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(sent(api).place).toEqual({ pageIndex: 0, corner: "topLeft" });
  });

  it("can sign so that nothing shows on the page", async () => {
    const api = fakeApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await fillIn(user, dialog);
    await user.click(within(dialog).getByRole("radio", { name: t.invisible }));
    expect(within(dialog).queryByLabelText(t.cornerLabel)).toBeNull();
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(sent(api).place).toBeNull();
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
    expect(api.listCertificates).toHaveBeenCalledTimes(2);
  });
});

describe("signing with a certificate of this computer (B3-06)", () => {
  it("signs with the certificate on a card by its number alone, and asks for no password", async () => {
    const api = fakeApi("ready", undefined, [CARD, EXPIRED]);
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    // This computer's certificates come first when one can sign, and it is chosen.
    expect(await within(dialog).findByRole("radio", { name: CARD.subject })).toBeChecked();
    expect(within(dialog).getByRole("radio", { name: t.sourceStore })).toBeChecked();
    expect(within(dialog).queryByLabelText(t.passwordLabel)).toBeNull();
    expect(within(dialog).getByText(t.pinNote)).toBeInTheDocument();
    expect(within(dialog).getByText(t.certificateDetail(CARD.issuer, "2030-05-14"))).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: t.start }));

    const args = sent(api);
    expect(args.key).toEqual({ kind: "store", certificate: 7 });
    expect(JSON.stringify(args)).not.toMatch(/password|pin/i);
    expect(await within(dialog).findByRole("status")).toHaveTextContent(t.result("報告（已簽署）.pdf"));
  });

  it("shows why a certificate cannot sign, and does not let it be chosen", async () => {
    const api = fakeApi("ready", undefined, [CARD, EXPIRED]);
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    const expired = await within(dialog).findByRole("radio", { name: EXPIRED.subject });
    expect(expired).toBeDisabled();
    expect(expired).toHaveAccessibleDescription(
      `${t.certificateDetail(EXPIRED.issuer, "2024-01-02")}（${t.problems.expired}）`,
    );
  });

  it("turns to a certificate file when this computer has no certificate that signs", async () => {
    const api = fakeApi("ready", undefined, [EXPIRED]);
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    expect(await within(dialog).findByRole("button", { name: t.chooseFile })).toBeInTheDocument();
    expect(within(dialog).getByRole("radio", { name: t.sourceFile })).toBeChecked();
    // Asked for: what there is, and why none of it signs.
    await user.click(within(dialog).getByRole("radio", { name: t.sourceStore }));
    expect(within(dialog).getByText(t.noCertificates)).toBeInTheDocument();
    expect(within(dialog).getByRole("radio", { name: EXPIRED.subject })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: t.start })).toBeDisabled();
  });

  it("lists the certificates again when asked, after a card is put in", async () => {
    const api = fakeApi("ready");
    api.listCertificates.mockResolvedValueOnce([]).mockResolvedValueOnce([CARD]);
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await user.click(await within(dialog).findByRole("radio", { name: t.sourceStore }));
    await user.click(within(dialog).getByRole("button", { name: t.refresh }));
    expect(await within(dialog).findByRole("radio", { name: CARD.subject })).toBeChecked();
    expect(api.listCertificates).toHaveBeenCalledTimes(2);
    expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled();
  });

  it("says nothing when the PIN was not given, and says so when the card was taken out", async () => {
    const api = fakeApi("ready", () => Promise.reject({ code: "cancelled", message: "" }), [CARD]);
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    const start = await within(dialog).findByRole("button", { name: t.start });
    await waitFor(() => expect(start).toBeEnabled());
    await user.click(start);
    expect(api.sign).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(start).toBeEnabled());
    expect(within(dialog).queryByRole("alert")).toBeNull();

    api.sign.mockRejectedValueOnce({ code: "certificateUnavailable", message: "" });
    await user.click(start);
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      strings.error.messages.certificateUnavailable,
    );
  });

  it("reads the certificates once more, and says so, when that failed", async () => {
    const api = fakeApi("ready");
    api.listCertificates.mockRejectedValueOnce({ code: "internal", message: "" });
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await user.click(await within(dialog).findByRole("radio", { name: t.sourceStore }));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(t.listFailed);
    await user.click(within(dialog).getByRole("button", { name: t.refresh }));
    expect(await within(dialog).findByText(t.noCertificates)).toBeInTheDocument();
  });
});

describe("visible signature with saved picture (#744)", () => {
  it("lists saved signature pictures with thumbnails and sends the selected id", async () => {
    const api = fakeApi("ready", undefined, [CARD]);
    const sigApi = fakeSignaturesApi([
      { id: 7, width: 120, height: 60 },
      { id: 9, width: 140, height: 60 },
    ]);
    const { user } = renderShell(api, false, sigApi);
    const dialog = await openDialog(user);
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());

    const checkbox = within(dialog).getByRole("checkbox", { name: t.usePicture });
    expect(checkbox).not.toBeChecked();
    await user.click(checkbox);

    const first = await within(dialog).findByRole("radio", { name: t.pictureItemLabel(0) });
    const second = within(dialog).getByRole("radio", { name: t.pictureItemLabel(1) });
    expect(first).toBeChecked();
    await waitFor(() => expect(sigApi.getSignatureImagePng).toHaveBeenCalledWith(7));
    expect(sigApi.getSignatureImagePng).toHaveBeenCalledWith(9);

    await user.click(second);
    await user.click(within(dialog).getByRole("button", { name: t.start }));

    expect(sent(api).place).toEqual({
      pageIndex: 0,
      corner: "bottomRight",
      signatureImage: 9,
    });
  });

  it("opens the #587 creation dialog when none exist, auto-selects the created signature, and enables signing", async () => {
    const api = fakeApi("ready", undefined, [CARD]);
    const sigApi = fakeSignaturesApi([]);
    const { user } = renderShell(api, false, sigApi);
    const dialog = await openDialog(user);
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());

    const checkbox = within(dialog).getByRole("checkbox", { name: t.usePicture });
    await user.click(checkbox);
    expect(await within(dialog).findByText(t.noPictures)).toBeInTheDocument();
    const start = within(dialog).getByRole("button", { name: t.start });
    expect(start).toBeDisabled();

    await user.click(within(dialog).getByRole("button", { name: t.createPicture }));
    const createDialog = await screen.findByRole("dialog", { name: t.createPictureTitle });
    await user.click(within(createDialog).getByRole("button", { name: strings.savedSignatures.importTab }));
    await user.click(within(createDialog).getByRole("button", { name: strings.savedSignatures.importButton }));

    await waitFor(() => expect(sigApi.importSignatureImage).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.createPictureTitle })).toBeNull());

    const created = await within(dialog).findByRole("radio", { name: t.pictureItemLabel(0) });
    expect(created).toBeChecked();
    expect(created).toHaveFocus();
    await waitFor(() => expect(start).toBeEnabled());

    await user.click(start);
    expect(sent(api).place).toEqual({
      pageIndex: 0,
      corner: "bottomRight",
      signatureImage: 1,
    });
  });

  it("disables picture controls and omits signatureImage when invisible is selected", async () => {
    const api = fakeApi("ready", undefined, [CARD]);
    const sigApi = fakeSignaturesApi([{ id: 7, width: 120, height: 60 }]);
    const { user } = renderShell(api, false, sigApi);
    const dialog = await openDialog(user);
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());

    const checkbox = within(dialog).getByRole("checkbox", { name: t.usePicture });
    await user.click(checkbox);
    const first = await within(dialog).findByRole("radio", { name: t.pictureItemLabel(0) });
    expect(first).toBeEnabled();

    await user.click(within(dialog).getByRole("radio", { name: t.invisible }));
    expect(checkbox).toBeDisabled();
    expect(first).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled();

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(sent(api).place).toBeNull();
  });

  it("supports keyboard toggling, selection, and error display for unavailable picture", async () => {
    const api = fakeApi("ready", () => Promise.reject({ code: "signatureImageUnavailable", message: "" }), [CARD]);
    const sigApi = fakeSignaturesApi([
      { id: 3, width: 120, height: 60 },
      { id: 4, width: 120, height: 60 },
    ]);
    const { user } = renderShell(api, false, sigApi);
    const dialog = await openDialog(user);
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());

    const checkbox = within(dialog).getByRole("checkbox", { name: t.usePicture });
    checkbox.focus();
    await user.keyboard(" ");
    expect(checkbox).toBeChecked();

    const first = await within(dialog).findByRole("radio", { name: t.pictureItemLabel(0) });
    const second = within(dialog).getByRole("radio", { name: t.pictureItemLabel(1) });
    first.focus();
    await user.keyboard("{ArrowDown}");
    expect(second).toBeChecked();

    // Opening and cancelling the #587 creator keeps SignDialog open and restores focus.
    const addButton = within(dialog).getByRole("button", { name: t.addPicture });
    await user.click(addButton);
    const createDialog = await screen.findByRole("dialog", { name: t.createPictureTitle });
    await user.click(within(createDialog).getByRole("button", { name: t.createPictureCancel }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.createPictureTitle })).toBeNull());
    expect(addButton).toHaveFocus();

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      strings.error.messages.signatureImageUnavailable,
    );
  });
});
