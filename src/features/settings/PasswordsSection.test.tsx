import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { OpenNotice } from "@/features/open/OpenNotice";
import { PasswordsSection, type PasswordsApi } from "@/features/settings/PasswordsSection";
import { strings } from "@/i18n/zh-TW";
import type { RememberedPasswords } from "@/ipc/generated/contract";

const t = strings.settings.passwords;

function fakeApi(counts: RememberedPasswords[], forgotten = 0) {
  const queue = [...counts];
  return {
    rememberedPasswords: vi.fn<PasswordsApi["rememberedPasswords"]>(() =>
      Promise.resolve(queue.length > 1 ? queue.shift()! : queue[0]!),
    ),
    forgetAllPasswords: vi.fn<PasswordsApi["forgetAllPasswords"]>(() => Promise.resolve(forgotten)),
  } satisfies PasswordsApi;
}

describe("the remembered passwords in the settings (B2-26)", () => {
  it("says how many there are and how many there can be, and has no list", async () => {
    render(<PasswordsSection api={fakeApi([{ count: 3, limit: 256 }])} />);
    expect(await screen.findByText(t.count(3))).toBeVisible();
    expect(screen.getByText(t.limit(256))).toBeVisible();
    // An entry has no file name to list it by; the note says so, and where the passwords are.
    expect(screen.getByText(t.note)).toBeVisible();
    expect(screen.queryByText(t.full)).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: t.title })).toBeVisible();
  });

  it("has nothing to forget when none is remembered", async () => {
    render(<PasswordsSection api={fakeApi([{ count: 0, limit: 256 }])} />);
    expect(await screen.findByText(t.none)).toBeVisible();
    expect(screen.getByRole("button", { name: t.forgetAll })).toBeDisabled();
  });

  it("says when no more can be remembered", async () => {
    render(<PasswordsSection api={fakeApi([{ count: 256, limit: 256 }])} />);
    expect(await screen.findByText(t.full)).toBeVisible();
  });

  it("asks first, forgets all of them when told to, and counts again", async () => {
    const api = fakeApi([{ count: 2, limit: 256 }, { count: 0, limit: 256 }], 2);
    const user = userEvent.setup();
    render(<PasswordsSection api={api} />);
    await screen.findByText(t.count(2));

    // Asked, and the question can be turned down: nothing is forgotten.
    await user.click(screen.getByRole("button", { name: t.forgetAll }));
    const question = screen.getByRole("group", { name: t.confirmTitle });
    expect(question).toHaveTextContent(t.confirmText(2));
    // The safe answer has the focus.
    expect(screen.getByRole("button", { name: t.cancel })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: t.cancel }));
    expect(api.forgetAllPasswords).not.toHaveBeenCalled();
    expect(screen.queryByRole("group", { name: t.confirmTitle })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: t.forgetAll }));
    await user.click(screen.getByRole("button", { name: t.confirm }));
    expect(api.forgetAllPasswords).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(t.forgotAll(2))).toBeVisible();
    expect(await screen.findByText(t.none)).toBeVisible();
  });

  it("says when the Credential Manager cannot be used", async () => {
    const api = fakeApi([{ count: 1, limit: 256 }]);
    api.rememberedPasswords.mockRejectedValueOnce({ code: "credentialsUnavailable", message: "" });
    render(<PasswordsSection api={api} />);
    expect(await screen.findByRole("alert")).toHaveTextContent(t.failed);
  });
});

describe("the notice that a password was not kept as asked (B2-26)", () => {
  it("says why, for every reason", () => {
    for (const reason of ["noKey", "full", "unsuitable", "unavailable"] as const) {
      const { unmount } = render(<OpenNotice notice={{ kind: "notRemembered", reason }} onDismiss={vi.fn()} />);
      expect(screen.getByRole("status")).toHaveTextContent(strings.password.notRemembered[reason]);
      unmount();
    }
  });
});
