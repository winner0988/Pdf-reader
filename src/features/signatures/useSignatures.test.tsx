import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useSignatures, type SignaturesApi } from "@/features/signatures/useSignatures";
import type { SignatureInfo, SignatureReport } from "@/ipc/generated/contract";

function signature(overrides: Partial<SignatureInfo> = {}): SignatureInfo {
  return {
    status: "valid",
    signerTrusted: false,
    reason: null,
    fieldName: "Signature1",
    signer: "Jane Public",
    claimedTime: null,
    certification: null,
    ...overrides,
  };
}

const report = (...signatures: SignatureInfo[]): SignatureReport => ({ signatures, truncated: false });

function api(answer: (doc: number) => Promise<SignatureReport>) {
  return { getSignatures: vi.fn<SignaturesApi["getSignatures"]>(answer) } satisfies SignaturesApi;
}

describe("verifying the signatures of the document shown", () => {
  it("asks for the signatures of a document that is as its file is", async () => {
    const source = api(() => Promise.resolve(report(signature())));
    const { result } = renderHook(() => useSignatures(source, 5, 1, false));
    expect(result.current).toEqual({ status: "none" });
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(source.getSignatures).toHaveBeenCalledWith(5);
  });

  it("shows nothing for a document without signatures, or when the check failed", async () => {
    const none = api(() => Promise.resolve(report()));
    const hook = renderHook(() => useSignatures(none, 5, 1, false));
    await waitFor(() => expect(none.getSignatures).toHaveBeenCalled());
    expect(hook.result.current).toEqual({ status: "none" });

    const failing = api(() => Promise.reject(new Error("the engine stopped")));
    const failed = renderHook(() => useSignatures(failing, 5, 1, false));
    await waitFor(() => expect(failing.getSignatures).toHaveBeenCalled());
    expect(failed.result.current).toEqual({ status: "none" });
  });

  it("keeps the answer while the document has changes of its own, and asks again once saved", async () => {
    const source = api(() => Promise.resolve(report(signature())));
    const { result, rerender } = renderHook(({ doc, unsaved }) => useSignatures(source, doc, 1, unsaved), {
      initialProps: { doc: 5, unsaved: false },
    });
    await waitFor(() => expect(result.current.status).toBe("ready"));
    // An edit gives the document a new id: the signatures are those of the file, which is as it was.
    rerender({ doc: 6, unsaved: true });
    expect(result.current.status).toBe("ready");
    expect(source.getSignatures).toHaveBeenCalledTimes(1);
    // Saving writes the file: its signatures are asked again.
    rerender({ doc: 7, unsaved: false });
    await waitFor(() => expect(source.getSignatures).toHaveBeenCalledTimes(2));
    expect(source.getSignatures).toHaveBeenLastCalledWith(7);
  });

  it("starts afresh for another file, and ignores a late answer for one that is gone", async () => {
    const answers = new Map<number, (report: SignatureReport) => void>();
    const source = api((doc) => new Promise((resolve) => answers.set(doc, resolve)));
    const { result, rerender } = renderHook(({ doc, session }) => useSignatures(source, doc, session, false), {
      initialProps: { doc: 5, session: 1 },
    });
    rerender({ doc: 8, session: 2 });
    await waitFor(() => expect(answers.size).toBe(2));
    // The answer for the first file comes late.
    act(() => answers.get(5)!(report(signature())));
    expect(result.current).toEqual({ status: "none" });
    act(() => answers.get(8)!(report(signature({ status: "invalid", signer: null }))));
    await waitFor(() => expect(result.current.status).toBe("ready"));
  });
});
