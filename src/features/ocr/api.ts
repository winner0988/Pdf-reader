// Recognising the text of scanned pages (B2-10, docs/architecture/ocr.md): the languages it can
// use, and starting, stopping and pointing it. How it goes arrives as open events.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, LanguageImport, OcrLanguages } from "@/ipc/generated/contract";

export type OcrApi = {
  /** The languages installed: those that came with the app, and the ones the user imported. */
  languages(): Promise<OcrLanguages>;
  /** The main process asks for a `.traineddata` file (the path never reaches the page) and imports it. */
  importLanguage(): Promise<LanguageImport>;
  /** Removes a language the user imported; resolves with the languages there are now. */
  removeLanguage(code: string): Promise<OcrLanguages>;
  /** Recognises the document's scanned pages now, whatever the settings say about doing it by itself. */
  start(doc: DocumentId): Promise<void>;
  /** Stops it; what was recognised stays. */
  stop(doc: DocumentId): Promise<void>;
  /** The page the user looks at, which is recognised first. */
  focus(doc: DocumentId, pageIndex: number): Promise<void>;
};

export const tauriOcrApi: OcrApi = {
  languages: () => invoke<OcrLanguages>("get_ocr_languages"),
  importLanguage: () => invoke<LanguageImport>("import_ocr_language"),
  removeLanguage: (code) => invoke<OcrLanguages>("remove_ocr_language", { args: { code } }),
  start: (doc) => invoke<void>("start_ocr", { args: { doc } }),
  stop: (doc) => invoke<void>("stop_ocr", { args: { doc } }),
  focus: (doc, pageIndex) => invoke<void>("set_ocr_focus", { args: { doc, pageIndex } }),
};
