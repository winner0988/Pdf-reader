// What the page knows of recognising the text of each tab's scanned pages (B2-10): the latest
// progress, and how many times each page of that document had its text recognised, which tells
// the page to ask for the text again.

import { strings } from "@/i18n";
import { LIMITS, type DocumentId, type OcrProgress, type OpenEvent, type TabId } from "@/ipc/generated/contract";

/** ASCII/C1 control characters and invisible bidi/format characters rejected by `ipc_contract::text::check_clean`. */
function isDisallowedWordChar(char: string): boolean {
  const code = char.codePointAt(0);
  if (code === undefined) return false;
  return (
    code <= 0x001f ||
    (code >= 0x007f && code <= 0x009f) ||
    code === 0x061c ||
    (code >= 0x200b && code <= 0x200f) ||
    (code >= 0x202a && code <= 0x202e) ||
    (code >= 0x2060 && code <= 0x2069) ||
    code === 0xfeff
  );
}

const utf8Encoder = new TextEncoder();

/** Client-side check matching `validate::check_recognised_word_text` before sending an edit. */
export function validateRecognisedWordText(text: string): string | null {
  const t = strings.ocr.correct;
  if (text.trim() === "") return t.emptyError;
  if (utf8Encoder.encode(text).length > LIMITS.maxRecognisedWordBytes) return t.tooLongError;
  for (const ch of text) {
    if (isDisallowedWordChar(ch)) return t.controlCharError;
  }
  return null;
}

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
