import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SensitiveApi } from "@/features/sensitive/api";
import { fitsClipboard } from "@/features/sensitive/api";
import { SensitiveDialog } from "@/features/sensitive/SensitiveDialog";
import { SensitiveSection } from "@/features/sensitive/SensitiveSection";
import { strings } from "@/i18n/zh-TW";
import type { ErrorCode, MarkCheck, SensitiveFile } from "@/ipc/generated/contract";

const t = strings.sensitive;

function rejection(code: ErrorCode) {
  return Promise.reject({ code, message: "x" });
}

function dialog(check: MarkCheck, options: { sensitive?: boolean; set?: SensitiveApi["set"] } = {}) {
  const api = {
    check: vi.fn(() => Promise.resolve(check)),
    set: vi.fn(options.set ?? (() => Promise.resolve())),
  } satisfies Pick<SensitiveApi, "check" | "set">;
  const onOpenChange = vi.fn();
  const onFinished = vi.fn();
  const utils = render(
    <SensitiveDialog
      open
      onOpenChange={onOpenChange}
      doc={7}
      sensitive={options.sensitive ?? false}
      api={api}
      onFinished={onFinished}
    />,
  );
  return { api, onOpenChange, onFinished, user: userEvent.setup(), ...utils };
}

describe("the sensitive mark (B3-05)", () => {
  it("says what marking does and what happens to the file, and writes it once confirmed", async () => {
    const { api, onOpenChange, onFinished, user } = dialog("ready");
    expect(screen.getByRole("dialog", { name: t.markTitle })).toBeInTheDocument();
    expect(screen.getByText(t.checking)).toBeInTheDocument();
    for (const item of t.whatItDoes) expect(screen.getByText(item)).toBeInTheDocument();
    expect(await screen.findByText(t.ready)).toBeInTheDocument();
    expect(api.check).toHaveBeenCalledWith(7);
    expect(api.set).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: t.mark }));
    expect(api.set).toHaveBeenCalledWith(7, true, null);
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
    expect(onFinished).toHaveBeenCalledWith(t.marked);
  });

  it("takes the mark out of a sensitive document", async () => {
    const { api, onFinished, user } = dialog("ready", { sensitive: true });
    expect(screen.getByRole("dialog", { name: t.unmarkTitle })).toBeInTheDocument();
    expect(screen.getByText(t.whatUnmarkDoes)).toBeInTheDocument();
    await user.click(await screen.findByRole("button", { name: t.unmark }));
    expect(api.set).toHaveBeenCalledWith(7, false, null);
    await waitFor(() => expect(onFinished).toHaveBeenCalledWith(t.unmarked));
  });

  it.each([
    ["signed", t.signed],
    ["readOnly", t.readOnly],
    ["unsaved", t.unsaved],
    ["changedOnDisk", t.changedOnDisk],
  ] as const)("refuses a file that is %s, says why, and offers nothing to confirm", async (check, reason) => {
    const { api } = dialog(check);
    expect(await screen.findByText(reason)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: t.mark })).not.toBeInTheDocument();
    expect(screen.queryByText(t.ready)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: t.close })).toBeInTheDocument();
    expect(api.set).not.toHaveBeenCalled();
  });

  it("asks an encrypted file's permissions password, sends it once, and never keeps it", async () => {
    let attempt = 0;
    const { api, onFinished, user } = dialog("needsPassword", {
      set: () => (++attempt === 1 ? rejection("notAllowed") : attempt === 2 ? rejection("encrypted") : Promise.resolve()),
    });
    expect(await screen.findByText(t.needsPassword)).toBeInTheDocument();
    const mark = screen.getByRole("button", { name: t.mark });
    expect(mark).toBeDisabled();
    const field = screen.getByLabelText(t.passwordLabel);
    expect(field).toHaveAttribute("type", "password");

    await user.type(field, "user");
    await user.click(mark);
    expect(api.set).toHaveBeenLastCalledWith(7, true, "user");
    expect(await screen.findByText(t.notOwnerPassword)).toBeInTheDocument();
    expect(field).toHaveValue("");

    await user.type(field, "wrong");
    await user.keyboard("{Enter}");
    expect(await screen.findByText(t.wrongPassword)).toBeInTheDocument();
    expect(field).toHaveValue("");

    await user.type(field, "owner");
    await user.click(mark);
    expect(api.set).toHaveBeenLastCalledWith(7, true, "owner");
    await waitFor(() => expect(onFinished).toHaveBeenCalledWith(t.marked));
  });

  it.each([
    ["fileInUse", t.fileInUse],
    ["corrupted", t.repaired],
    ["readOnly", t.readOnly],
    ["internal", t.failed],
  ] as const)("says why writing failed (%s), and the dialog stays", async (code, message) => {
    const { onOpenChange, user } = dialog("ready", { set: () => rejection(code) });
    await user.click(await screen.findByRole("button", { name: t.mark }));
    expect(await screen.findByRole("alert")).toHaveTextContent(message);
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
  });

  it("says when the file could not be checked", async () => {
    const api = { check: vi.fn(() => rejection("workerCrashed")), set: vi.fn() };
    render(<SensitiveDialog open onOpenChange={vi.fn()} doc={1} sensitive={false} api={api} onFinished={vi.fn()} />);
    expect(await screen.findByText(t.checkFailed)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: t.mark })).not.toBeInTheDocument();
  });
});

describe("the list of files marked on this computer (B3-05)", () => {
  const file = (id: number, displayName: string, open = false): SensitiveFile => ({ id, displayName, open });

  it("lists the names, says which are open, and takes one off the list", async () => {
    const api = {
      list: vi.fn(() => Promise.resolve([file(1, "合約.pdf", true), file(2, "薪資.pdf")])),
      remove: vi.fn(() => Promise.resolve([file(1, "合約.pdf", true)])),
    };
    const user = userEvent.setup();
    render(<SensitiveSection api={api} />);
    expect(await screen.findByText("合約.pdf")).toBeInTheDocument();
    expect(screen.getByText("薪資.pdf")).toBeInTheDocument();
    expect(screen.getAllByText(t.listOpen)).toHaveLength(1);
    expect(screen.getByText(t.listNote)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: t.listRemove("薪資.pdf") }));
    expect(api.remove).toHaveBeenCalledWith(2);
    await waitFor(() => expect(screen.queryByText("薪資.pdf")).not.toBeInTheDocument());
  });

  it("says when there are none, or when the list cannot be read", async () => {
    const empty = { list: vi.fn(() => Promise.resolve([])), remove: vi.fn() };
    const { unmount } = render(<SensitiveSection api={empty} />);
    expect(await screen.findByText(t.listNone)).toBeInTheDocument();
    unmount();
    const failing = { list: vi.fn(() => rejection("internal")), remove: vi.fn() };
    render(<SensitiveSection api={failing} />);
    expect(await screen.findByRole("alert")).toHaveTextContent(t.listFailed);
  });
});

describe("the size of a protected copy", () => {
  it("counts UTF-8 bytes against the limit", () => {
    expect(fitsClipboard("abc", 3)).toBe(true);
    expect(fitsClipboard("abcd", 3)).toBe(false);
    // Three bytes a character.
    expect(fitsClipboard("機密", 6)).toBe(true);
    expect(fitsClipboard("機密", 5)).toBe(false);
  });
});
