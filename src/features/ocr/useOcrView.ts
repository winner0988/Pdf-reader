// What one tab's reader shows and does about recognising the text of scanned pages (B2-10): the
// status bar's progress, the note on a page whose text was recognised, the menu item, the hints
// when it ends, and telling the main process which page is looked at.

import { useCallback, useEffect, useRef, useState } from "react";

import type { OcrApi } from "@/features/ocr/api";
import type { OcrTab } from "@/features/ocr/model";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId, OcrRun } from "@/ipc/generated/contract";

/** The page is looked at this long before the main process is told: scrolling past is not looking. */
export const FOCUS_DELAY_MS = 300;

type Options = {
  api?: OcrApi;
  ocr?: OcrTab;
  /** The tab's document, once it is open. */
  doc?: DocumentId;
  /** 1-based, as the status bar shows it. */
  currentPage: number;
  /** The shell shown: a hidden tab's reader does not point the main process at its page. */
  active: boolean;
  showHint: (text: string) => void;
};

export function useOcrView({ api, ocr, doc, currentPage, active, showHint }: Options) {
  const progress = ocr?.progress ?? null;
  // Only what is about the document shown: after an edit it takes a moment to hear of the new one.
  const current = progress !== null && progress.doc === doc ? progress : null;
  const running = current?.run === "running";

  // Which pages of this document have text that was recognised (their text arrived and said so).
  const [recognised, setRecognised] = useState<{ doc: DocumentId | undefined; pages: ReadonlySet<number> }>({
    doc,
    pages: new Set(),
  });
  if (recognised.doc !== doc) setRecognised({ doc, pages: new Set() });
  const onRecognised = useCallback(
    (pageIndex: number, yes: boolean) =>
      setRecognised((before) => {
        if (before.pages.has(pageIndex) === yes) return before;
        const pages = new Set(before.pages);
        if (yes) pages.add(pageIndex);
        else pages.delete(pageIndex);
        return { doc: before.doc, pages };
      }),
    [],
  );

  // What it came to is told once, when it ends; the user's own start is told even if nothing needed it.
  const told = useRef({ run: current?.run as OcrRun | undefined, manual: false });
  const hint = useRef(showHint);
  useEffect(() => {
    hint.current = showHint;
  });
  const run = current?.run;
  useEffect(() => {
    const before = told.current.run;
    told.current.run = run;
    if (current === null || run === before) return;
    const manual = told.current.manual;
    if (run !== "running") told.current.manual = false;
    if (run === "done" && before === "running") {
      if (current.scans > 0) hint.current(strings.ocr.done(current.recognised, current.failed));
      else if (manual) hint.current(strings.ocr.none);
    } else if (run === "stopped" && before === "running") {
      hint.current(strings.ocr.stopped);
    } else if (run === "noLanguage") {
      hint.current(strings.ocr.noLanguage);
    } else if (run === "failed") {
      hint.current(strings.ocr.failed);
    }
  }, [run, current]);

  // The page the user looks at is read first.
  useEffect(() => {
    if (!api || doc === undefined || !active) return;
    const timer = window.setTimeout(() => api.focus(doc, currentPage - 1).catch(() => {}), FOCUS_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [api, doc, active, currentPage]);

  const start = () => {
    if (!api || doc === undefined) return;
    told.current.manual = true;
    api.focus(doc, currentPage - 1).catch(() => {});
    api.start(doc).catch(() => hint.current(strings.ocr.failed));
  };
  const stop = () => {
    if (api && doc !== undefined) api.stop(doc).catch(() => {});
  };

  let versionTotal = 0;
  for (const version of ocr?.doc === doc ? (ocr?.versions.values() ?? []) : []) versionTotal += version;

  return {
    /** Which pages' text was recognised since the document opened, for the pages to ask again. */
    textVersions: ocr?.doc === doc ? ocr?.versions : undefined,
    /** How many pages were recognised in all: a search that was done is done again when it grows. */
    versionTotal,
    onRecognised,
    /** In the status bar while scanned pages are being read (some were found). */
    status:
      running && current.scans > 0
        ? { text: strings.ocr.running(current.recognised + current.failed, current.scans), onStop: stop }
        : undefined,
    /** In the status bar next to a page whose text was recognised. */
    note: recognised.pages.has(currentPage - 1) ? strings.ocr.pageNote : null,
    /** The "⋯" menu item: only where the main process can recognise. */
    menu: api && doc !== undefined ? { running, onStart: start } : undefined,
    /** What to say when the user tries to select text on a page that has none. */
    noText: () =>
      showHint(
        current === null
          ? strings.text.noTextLayer
          : current.run === "running"
            ? strings.ocr.selectWhileReading
            : current.run === "idle" || current.run === "stopped" || current.run === "noLanguage"
              ? strings.ocr.selectNotStarted
              : strings.text.noTextLayer,
      ),
  };
}
