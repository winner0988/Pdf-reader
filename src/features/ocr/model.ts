// What the page knows of recognising the text of each tab's scanned pages (B2-10): the latest
// progress, and how many times each page of that document had its text recognised, which tells
// the page to ask for the text again.

import type { DocumentId, OcrProgress, OpenEvent, TabId } from "@/ipc/generated/contract";

export type OcrEvent = Extract<OpenEvent, { kind: "ocr" } | { kind: "ocrPage" }>;

export function isOcrEvent(event: OpenEvent): event is OcrEvent {
  return event.kind === "ocr" || event.kind === "ocrPage";
}

export type OcrTab = {
  /** The latest the main process said; null until it has said anything. */
  progress: OcrProgress | null;
  /** The document `versions` count for: the tab's latest one. */
  doc: DocumentId | null;
  /** By page index: how often its text was recognised while the tab showed `doc`. */
  versions: ReadonlyMap<number, number>;
};

export type OcrState = ReadonlyMap<TabId, OcrTab>;

export const noOcr: OcrState = new Map();

export type OcrAction = { type: "event"; event: OcrEvent } | { type: "closed"; tab: TabId };

const nothing: OcrTab = { progress: null, doc: null, versions: new Map() };

export function reduceOcr(state: OcrState, action: OcrAction): OcrState {
  const next = new Map(state);
  if (action.type === "closed") {
    next.delete(action.tab);
    return next;
  }
  const { event } = action;
  const before = state.get(event.tab) ?? nothing;
  if (event.kind === "ocr") {
    // Another document (the tab was edited): the counts were about the one before.
    const sameDocument = before.doc === event.progress.doc;
    next.set(event.tab, {
      progress: event.progress,
      doc: event.progress.doc,
      versions: sameDocument ? before.versions : new Map(),
    });
    return next;
  }
  const versions = new Map(before.doc === event.doc ? before.versions : []);
  versions.set(event.pageIndex, (versions.get(event.pageIndex) ?? 0) + 1);
  next.set(event.tab, { progress: before.progress, doc: event.doc, versions });
  return next;
}
