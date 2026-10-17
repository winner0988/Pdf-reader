import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { BatchApi } from "@/features/batch/api";
import { BatchHost } from "@/features/batch/BatchHost";
import { describeOutcome, reasonFor } from "@/features/batch/messages";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { BatchEvent, BatchOperation, BatchReport } from "@/ipc/generated/contract";

const t = strings.batch;
const e = strings.encryption;

/** A batch API whose events and end the test decides. */
function fakeBatchApi() {
  const state: {
    onEvent: (event: BatchEvent) => void;
    finish: (report: BatchReport | null) => void;
    fail: (error: unknown) => void;
  } = { onEvent: () => {}, finish: () => {}, fail: () => {} };
  const api = {
    run: vi.fn<BatchApi["run"]>((_operation: BatchOperation, onEvent) => {
      state.onEvent = onEvent;
      return {
        request: 41,
        done: new Promise<BatchReport | null>((resolve, reject) => {
          state.finish = resolve;
          state.fail = reject;
        }),
      };
    }),
    cancel: vi.fn<BatchApi["cancel"]>(() => Promise.resolve()),
  } satisfies BatchApi;
  return { api, state };
}

function renderShell(api?: BatchApi) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5 } };
  const user = userEvent.setup();
  render(
    <BatchHost api={api}>
      <ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} />
    </BatchHost>,
  );
  return { user };
}

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  await user.click(await screen.findByRole("menuitem", { name: strings.menu.batch }));
  return screen.findByRole("dialog", { name: t.title });
}

const FILES = ["a.pdf", "secret.pdf", "c.pdf"];

describe("a batch (B3-04)", () => {
  it("is in the menu, and disabled without the main process", async () => {
    const { user } = renderShell(undefined);
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    expect(await screen.findByRole("menuitem", { name: strings.menu.batch })).toHaveAttribute("aria-disabled", "true");
  });

  it("removes the metadata of each file and says how each went", async () => {
    const { api, state } = fakeBatchApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    for (const note of t.notes(500)) expect(dialog).toHaveTextContent(note);
    expect(within(dialog).getByRole("radio", { name: t.removeMetadata })).toBeChecked();

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(api.run).toHaveBeenCalledOnce();
    expect(api.run.mock.calls[0]![0]).toEqual({ kind: "removeMetadata" });
    // Nobody can close it or start another while the main process works.
    expect(within(dialog).queryByRole("button", { name: t.start })).toBeNull();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();

    act(() => state.onEvent({ kind: "started", files: FILES }));
    const list = within(dialog).getByRole("list", { name: t.filesLabel });
    expect(within(list).getAllByRole("listitem")).toHaveLength(3);
    expect(list).toHaveTextContent(t.waiting);
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.running(0, 3));

    act(() => {
      state.onEvent({ kind: "file", index: 0, outcome: { kind: "done", output: "a（隱私匯出）.pdf" } });
      state.onEvent({ kind: "file", index: 1, outcome: { kind: "failed", error: "encrypted" } });
    });
    expect(list).toHaveTextContent(t.done("a（隱私匯出）.pdf"));
    expect(list).toHaveTextContent(t.failedFile(t.reasons.encrypted!));
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.running(2, 3));

    act(() => state.onEvent({ kind: "file", index: 2, outcome: { kind: "done", output: "c（隱私匯出）.pdf" } }));
    await act(async () => state.finish({ done: 2, failed: 1, skipped: 0 }));
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.summary(2, 1, 0));
    await user.click(within(dialog).getByRole("button", { name: t.close }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("asks what an encrypted copy asks, sends the passwords once, and keeps none of them", async () => {
    const { api, state } = fakeBatchApi();
    const { user } = renderShell(api);
    let dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("radio", { name: t.encrypt }));
    // Nothing is asked for yet: it cannot start.
    expect(within(dialog).getByRole("button", { name: t.start })).toBeDisabled();
    expect(within(dialog).getByRole("status")).toHaveTextContent(e.problem.nothing);

    await user.type(within(dialog).getByLabelText(e.openPassword), "pw-7Qz");
    await user.type(within(dialog).getByLabelText(`${e.openPassword}（${e.again}）`), "pw-7Qz");
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(api.run.mock.calls[0]![0]).toEqual({
      kind: "encrypt",
      openPassword: "pw-7Qz",
      permissionsPassword: null,
      restrictions: { print: false, copy: false, modify: false },
    });
    act(() => state.onEvent({ kind: "started", files: ["a.pdf"] }));
    act(() => state.onEvent({ kind: "file", index: 0, outcome: { kind: "done", output: "a（已加密）.pdf" } }));
    await act(async () => state.finish({ done: 1, failed: 0, skipped: 0 }));
    await user.click(within(dialog).getByRole("button", { name: t.close }));

    // Opened again, nothing typed is there.
    dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("radio", { name: t.encrypt }));
    expect(within(dialog).getByLabelText(e.openPassword)).toHaveValue("");
  });

  it("takes the EXIF out of the pictures, and says what that is and is not", async () => {
    const { api, state } = fakeBatchApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    expect(dialog).not.toHaveTextContent(t.exifNote);
    await user.click(within(dialog).getByRole("radio", { name: t.removeExif }));
    expect(dialog).toHaveTextContent(t.exifNote);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(api.run.mock.calls[0]![0]).toEqual({ kind: "removeExif" });
    act(() => state.onEvent({ kind: "started", files: ["photo.pdf", "signed.pdf", "odd.pdf"] }));
    act(() => {
      state.onEvent({ kind: "file", index: 0, outcome: { kind: "done", output: "photo（已清除 EXIF）.pdf" } });
      state.onEvent({ kind: "file", index: 1, outcome: { kind: "failed", error: "invalidArgument" } });
      state.onEvent({ kind: "file", index: 2, outcome: { kind: "failed", error: "pictureNotCleaned" } });
    });
    const list = within(dialog).getByRole("list", { name: t.filesLabel });
    expect(list).toHaveTextContent(t.done("photo（已清除 EXIF）.pdf"));
    expect(list).toHaveTextContent(t.failedFile(t.reasons.invalidArgument!));
    expect(list).toHaveTextContent(t.failedFile(strings.error.messages.pictureNotCleaned));
  });

  it("stops before the next file when asked", async () => {
    const { api, state } = fakeBatchApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    act(() => state.onEvent({ kind: "started", files: FILES }));
    act(() => state.onEvent({ kind: "file", index: 0, outcome: { kind: "done", output: "a（隱私匯出）.pdf" } }));
    await user.click(within(dialog).getByRole("button", { name: t.stop }));
    expect(api.cancel).toHaveBeenCalledExactlyOnceWith(41);
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.stopping);
    expect(within(dialog).getByRole("button", { name: t.stop })).toBeDisabled();
    act(() => {
      state.onEvent({ kind: "file", index: 1, outcome: { kind: "skipped" } });
      state.onEvent({ kind: "file", index: 2, outcome: { kind: "skipped" } });
    });
    await act(async () => state.finish({ done: 1, failed: 0, skipped: 2 }));
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.summary(1, 0, 2));
    expect(within(dialog).getByRole("list", { name: t.filesLabel })).toHaveTextContent(t.skipped);
  });

  it("stays as it was when a system dialog is closed, and says when it could not start", async () => {
    const { api, state } = fakeBatchApi();
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await act(async () => state.finish(null));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());
    expect(within(dialog).queryByRole("alert")).toBeNull();

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await act(async () => state.fail({ code: "limitExceeded", message: "" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failed);
  });
});

describe("what a batch says of a file", () => {
  it("has a reason for each code, special or general", () => {
    expect(reasonFor("encrypted")).toBe(t.reasons.encrypted);
    expect(reasonFor("diskFull")).toBe(strings.error.messages.diskFull);
    expect(reasonFor("cancelled")).toBe(strings.error.messages.internal);
    expect(describeOutcome(null)).toBe(t.waiting);
    expect(describeOutcome({ kind: "skipped" })).toBe(t.skipped);
    expect(describeOutcome({ kind: "done", output: "x.pdf" })).toBe(t.done("x.pdf"));
    expect(t.summary(3, 0, 0)).toBe("完成 3 個。");
    expect(t.summary(1, 2, 3)).toBe("完成 1 個，失敗 2 個，略過 3 個。");
  });
});
