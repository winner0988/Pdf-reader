import { describe, expect, it } from "vitest";

import { initialSession, reduceSession, type OpenSession } from "@/features/open/session";
import type { DocumentInfo, OpenEvent } from "@/ipc/generated/contract";

const info: DocumentInfo = {
  doc: 4,
  displayName: "報告.pdf",
  pages: [
    { widthPt: 612, heightPt: 792 },
    { widthPt: 612, heightPt: 792 },
  ],
  hasOutline: false,
  security: { findings: [{ kind: "javaScript", count: 2 }], scanComplete: true },
};

const apply = (...events: OpenEvent[]): OpenSession =>
  events.reduce((session, event) => reduceSession(session, { type: "event", event }), initialSession);

describe("reduceSession", () => {
  it("shows loading, then the opened document", () => {
    const loading = apply({ kind: "opening", displayName: "報告.pdf" });
    expect(loading.shell).toEqual({ kind: "loading", displayName: "報告.pdf" });

    const opened = apply(
      { kind: "opening", displayName: "報告.pdf" },
      { kind: "opened", info, ignoredFiles: 0 },
    );
    expect(opened.doc).toBe(4);
    expect(opened.notice).toBeNull();
    expect(opened.shell).toEqual({
      kind: "open",
      document: {
        doc: 4,
        displayName: "報告.pdf",
        pages: info.pages,
        findings: [{ kind: "javaScript", count: 2 }],
      },
    });
  });

  it("shows the error with the file name", () => {
    const failed = apply({
      kind: "failed",
      displayName: "secret.pdf",
      error: { code: "encrypted", message: "" },
      ignoredFiles: 0,
    });
    expect(failed.shell).toEqual({ kind: "error", code: "encrypted", displayName: "secret.pdf" });
    expect(failed.doc).toBeNull();
  });

  it("tells the user when other dropped files were ignored", () => {
    const opened = apply({ kind: "opened", info, ignoredFiles: 2 });
    expect(opened.notice).toEqual({ kind: "dropMultiple", displayName: "報告.pdf" });

    const failed = apply({
      kind: "failed",
      displayName: "a.txt",
      error: { code: "notPdf", message: "" },
      ignoredFiles: 1,
    });
    expect(failed.notice).toEqual({ kind: "dropMultiple", displayName: "a.txt" });

    expect(reduceSession(opened, { type: "dismissNotice" }).notice).toBeNull();
    // The next open replaces the old notice.
    expect(reduceSession(opened, { type: "event", event: { kind: "opening", displayName: "b.pdf" } }).notice).toBeNull();
  });

  it("tracks drag hover and clears it when opening starts", () => {
    const hovering = apply({ kind: "dragHover", active: true });
    expect(hovering.dragActive).toBe(true);
    expect(hovering.shell).toEqual({ kind: "empty" });
    expect(apply({ kind: "dragHover", active: true }, { kind: "dragHover", active: false }).dragActive).toBe(false);
    expect(apply({ kind: "dragHover", active: true }, { kind: "opening", displayName: "a.pdf" }).dragActive).toBe(false);
  });

  it("closing returns to the empty state", () => {
    const closed = reduceSession(apply({ kind: "opened", info, ignoredFiles: 1 }), { type: "closed" });
    expect(closed).toEqual(initialSession);
  });

  it("a failing command shows its error code", () => {
    const failed = reduceSession(initialSession, { type: "failed", code: "internal" });
    expect(failed.shell).toEqual({ kind: "error", code: "internal" });
  });
});
