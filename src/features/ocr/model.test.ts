import { describe, expect, it } from "vitest";

import { noOcr, reduceOcr, type OcrAction } from "@/features/ocr/model";
import type { OcrProgress, TabId } from "@/ipc/generated/contract";

const tab = (n: number) => n as TabId;

const progress = (doc: number, patch: Partial<OcrProgress> = {}): OcrProgress => ({
  doc: doc as OcrProgress["doc"],
  run: "running",
  pages: 10,
  checked: 4,
  scans: 2,
  recognised: 1,
  failed: 0,
  ...patch,
});

const ocr = (n: number, p: OcrProgress): OcrAction => ({ type: "event", event: { kind: "ocr", tab: tab(n), progress: p } });
const page = (n: number, doc: number, pageIndex: number): OcrAction => ({
  type: "event",
  event: { kind: "ocrPage", tab: tab(n), doc: doc as OcrProgress["doc"], pageIndex },
});

describe("OCR state of the tabs (B2-10)", () => {
  it("keeps the latest progress of each tab", () => {
    let state = reduceOcr(noOcr, ocr(1, progress(5)));
    state = reduceOcr(state, ocr(2, progress(9, { run: "done" })));
    state = reduceOcr(state, ocr(1, progress(5, { recognised: 2 })));
    expect(state.get(tab(1))?.progress?.recognised).toBe(2);
    expect(state.get(tab(2))?.progress?.run).toBe("done");
  });

  it("counts the pages whose text was recognised, for the document they are about", () => {
    let state = reduceOcr(noOcr, ocr(1, progress(5)));
    state = reduceOcr(state, page(1, 5, 3));
    state = reduceOcr(state, page(1, 5, 3));
    state = reduceOcr(state, page(1, 5, 7));
    expect([...state.get(tab(1))!.versions]).toEqual([
      [3, 2],
      [7, 1],
    ]);
  });

  it("starts the counts again for another document of the tab (after an edit)", () => {
    let state = reduceOcr(noOcr, ocr(1, progress(5)));
    state = reduceOcr(state, page(1, 5, 3));
    state = reduceOcr(state, ocr(1, progress(6)));
    expect(state.get(tab(1))?.versions.size).toBe(0);
    expect(state.get(tab(1))?.doc).toBe(6);
  });

  it("is told about a page before the progress: the tab has no progress yet", () => {
    const state = reduceOcr(noOcr, page(4, 8, 0));
    expect(state.get(tab(4))).toEqual({ progress: null, doc: 8, versions: new Map([[0, 1]]) });
  });

  it("forgets a closed tab", () => {
    let state = reduceOcr(noOcr, ocr(1, progress(5)));
    state = reduceOcr(state, { type: "closed", tab: tab(1) });
    expect(state.has(tab(1))).toBe(false);
  });
});
