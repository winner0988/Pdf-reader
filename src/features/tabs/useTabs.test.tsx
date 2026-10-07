import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { OcrEvent } from "@/features/ocr/model";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import type { OpenApi } from "@/features/open/api";
import { useTabs } from "@/features/tabs/useTabs";
import type { DocumentId, DocumentInfo, OpenEvent, TabId } from "@/ipc/generated/contract";

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
};

function setup(onOcr?: (event: OcrEvent) => void) {
  let send: (event: OpenEvent) => void = () => {};
  const api = {
    listen: vi.fn((listener: (event: OpenEvent) => void) => {
      send = listener;
      return () => {};
    }),
    openDialog: vi.fn(() => Promise.resolve(true)),
    retry: vi.fn(() => Promise.resolve()),
    unlock: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
    setActive: vi.fn(() => Promise.resolve()),
  } satisfies OpenApi;
  const hook = renderHook(() => useTabs(api, onOcr));
  return { hook, send: (event: OpenEvent) => act(() => send(event)) };
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

  it("does not need anyone to listen to it", () => {
    const { hook, send } = setup();
    send({ kind: "opened", tab: 1 as TabId, info });
    send({ kind: "ocrPage", tab: 1 as TabId, doc: info.doc, pageIndex: 0 });
    expect(hook.result.current.latestInfo(1 as TabId)).toEqual(info);
  });
});
