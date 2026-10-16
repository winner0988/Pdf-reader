import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { OcrEvent } from "@/features/ocr/model";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import type { OpenApi } from "@/features/open/api";
import { useTabs } from "@/features/tabs/useTabs";
import type { DocumentId, DocumentInfo, OpenEvent, TabId, UnlockOutcome } from "@/ipc/generated/contract";

const info: DocumentInfo = {
  doc: 7 as DocumentId,
  displayName: "掃描.pdf",
  pages: [{ widthPt: 612, heightPt: 792 }],
  hasOutline: false,
  hasForm: false,
  security: { findings: [], scanComplete: true },
  permissions: ALL_PERMISSIONS,
  unsaved: false,
  encrypted: false,
  canUndo: false,
  canRedo: false,
  recovery: "none",
  passwordRemembered: false,
  passwordRecalled: false,
  sensitive: false,
};

function setup(onOcr?: (event: OcrEvent) => void, unlocked: UnlockOutcome = { notRemembered: null }) {
  let send: (event: OpenEvent) => void = () => {};
  const api = {
    listen: vi.fn((listener: (event: OpenEvent) => void) => {
      send = listener;
      return () => {};
    }),
    openDialog: vi.fn(() => Promise.resolve(true)),
    retry: vi.fn(() => Promise.resolve()),
    unlock: vi.fn(() => Promise.resolve(unlocked)),
    forgetPassword: vi.fn(() => Promise.resolve()),
    forgetAllPasswords: vi.fn(() => Promise.resolve(0)),
    rememberedPasswords: vi.fn(() => Promise.resolve({ count: 0, limit: 256 })),
    close: vi.fn(() => Promise.resolve()),
    setActive: vi.fn(() => Promise.resolve()),
  } satisfies OpenApi;
  const hook = renderHook(() => useTabs(api, onOcr));
  return { hook, api, send: (event: OpenEvent) => act(() => send(event)) };
}

describe("tabs and the open events", () => {
  it("hands how recognising scanned pages goes to the page, and leaves the tabs and what they last said alone", () => {
    const onOcr = vi.fn();
    const { hook, send } = setup(onOcr);
    send({ kind: "opened", tab: 1 as TabId, info });
    const tabs = hook.result.current.state.tabs;
    expect(hook.result.current.latestInfo(1 as TabId)).toEqual(info);

    const progress = { doc: info.doc, run: "running", pages: 1, checked: 1, scans: 1, recognised: 0, failed: 0 } as const;
    send({ kind: "ocr", tab: 1 as TabId, progress });
    send({ kind: "ocrPage", tab: 1 as TabId, doc: info.doc, pageIndex: 0 });
    expect(onOcr).toHaveBeenCalledTimes(2);
    expect(onOcr).toHaveBeenNthCalledWith(1, { kind: "ocr", tab: 1, progress });
    // The tab's document is still the one it last said, for a command that follows an edit.
    expect(hook.result.current.latestInfo(1 as TabId)).toEqual(info);
    expect(hook.result.current.state.tabs).toBe(tabs);
  });

  it("sends the password with what the user asked for it, and says when it was not kept as asked (B2-26)", async () => {
    const { hook, api } = setup(undefined, { notRemembered: "full" });
    await act(async () => hook.result.current.unlock(2 as TabId, "s3cret", true));
    expect(api.unlock).toHaveBeenCalledExactlyOnceWith(2, "s3cret", true);
    // The file opened, but the password was not kept: the user is told why.
    expect(hook.result.current.state.notice).toEqual({ kind: "notRemembered", reason: "full" });

    // Nothing to say when it was kept, or not asked for.
    const quiet = setup();
    await act(async () => quiet.hook.result.current.unlock(2 as TabId, "s3cret"));
    expect(quiet.api.unlock).toHaveBeenCalledExactlyOnceWith(2, "s3cret", false);
    expect(quiet.hook.result.current.state.notice).toBeNull();
  });

  it("forgets the password remembered for a tab's file, and reports when that fails (B2-26)", async () => {
    const { hook, api } = setup();
    await act(async () => hook.result.current.forgetPassword(3 as TabId));
    expect(api.forgetPassword).toHaveBeenCalledExactlyOnceWith(3);
    expect(hook.result.current.state.notice).toBeNull();

    api.forgetPassword.mockRejectedValueOnce({ code: "credentialsUnavailable", message: "" });
    await act(async () => hook.result.current.forgetPassword(3 as TabId));
    expect(hook.result.current.state.notice).toEqual({ kind: "failed", code: "credentialsUnavailable" });
  });

  it("does not need anyone to listen to it", () => {
    const { hook, send } = setup();
    send({ kind: "opened", tab: 1 as TabId, info });
    send({ kind: "ocrPage", tab: 1 as TabId, doc: info.doc, pageIndex: 0 });
    expect(hook.result.current.latestInfo(1 as TabId)).toEqual(info);
  });
});
