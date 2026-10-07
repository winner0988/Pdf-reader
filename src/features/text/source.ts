// A page's text from the main process (MVP-15, docs/architecture/text-selection.md). Pages near
// the view are asked for once and kept, the most recently used ones, while the document is open.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, PageText } from "@/ipc/generated/contract";

export type TextApi = {
  getPageText(doc: DocumentId, pageIndex: number): Promise<PageText>;
};

export const tauriTextApi: TextApi = {
  getPageText: (doc, pageIndex) => invoke<PageText>("get_page_text", { doc, pageIndex }),
};

export type TextSource = {
  /**
   * The page's text, kept for the next time. A different `version` than the kept text was asked
   * with asks again: a scanned page whose text was recognised since has new text (B2-10).
   */
  text(doc: DocumentId, pageIndex: number, version?: number): Promise<PageText>;
  /** The page's text if it has arrived: hit-testing a pointer cannot wait for it. */
  loaded(doc: DocumentId, pageIndex: number): PageText | undefined;
  /**
   * The page's text for copying a selection: from what is kept, else asked for without keeping
   * it, so that copying many pages does not push out the pages being read.
   */
  textOnce(doc: DocumentId, pageIndex: number): Promise<PageText>;
};

/** Pages kept: the ones on screen and around them, with room for going back and forth. */
const KEPT_PAGES = 64;

type Entry = { answer: Promise<PageText>; text?: PageText; version: number };

export function createTextSource(api: Pick<TextApi, "getPageText">, keep = KEPT_PAGES): TextSource {
  let cachedDoc: DocumentId | null = null;
  // In order of use, the most recent last.
  const pages = new Map<number, Entry>();
  const forDocument = (doc: DocumentId) => {
    // One document at a time: another one starts an empty cache.
    if (doc !== cachedDoc) {
      cachedDoc = doc;
      pages.clear();
    }
  };
  return {
    text(doc, pageIndex, version = 0) {
      forDocument(doc);
      const kept = pages.get(pageIndex);
      if (kept && kept.version === version) {
        pages.delete(pageIndex);
        pages.set(pageIndex, kept);
        return kept.answer;
      }
      const entry: Entry = { answer: api.getPageText(doc, pageIndex), version };
      pages.delete(pageIndex);
      pages.set(pageIndex, entry);
      entry.answer.then(
        (text) => {
          entry.text = text;
        },
        // A failure is not remembered: the page asks again the next time it is shown.
        () => {
          if (cachedDoc === doc && pages.get(pageIndex) === entry) pages.delete(pageIndex);
        },
      );
      for (const oldest of pages.keys()) {
        if (pages.size <= keep) break;
        pages.delete(oldest);
      }
      return entry.answer;
    },
    loaded(doc, pageIndex) {
      return doc === cachedDoc ? pages.get(pageIndex)?.text : undefined;
    },
    textOnce(doc, pageIndex) {
      const kept = doc === cachedDoc ? pages.get(pageIndex) : undefined;
      return kept ? kept.answer : api.getPageText(doc, pageIndex);
    },
  };
}
