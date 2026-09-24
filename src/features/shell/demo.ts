// Fake data for developing and screenshotting the shell before real IPC exists (MVP-05).
// Only used in development builds (see DevDemoSwitcher).

import type { ShellDocument, ShellState } from "@/features/shell/model";

const LETTER = { widthPt: 612, heightPt: 792 };

export const demoDocument: ShellDocument = {
  displayName: "報告.pdf",
  pages: Array.from({ length: 12 }, (_, index) =>
    index === 5 ? { widthPt: 842, heightPt: 595 } : LETTER,
  ),
  outline: [
    { title: "第 1 章 簡介", depth: 0, pageIndex: 0 },
    { title: "第 1.1 節 背景", depth: 1, pageIndex: 1 },
    { title: "第 1.2 節 目標", depth: 1, pageIndex: 2 },
    { title: "第 2 章 方法", depth: 0, pageIndex: 4 },
    { title: "第 2.1 節 資料", depth: 1, pageIndex: 5 },
    { title: "附錄", depth: 0, pageIndex: 10 },
  ],
  findings: [
    { kind: "javaScript", count: 2 },
    { kind: "openAction", count: 1 },
    { kind: "remoteFileSpec", count: 1 },
  ],
};

export const DEMO_STATES: Record<string, ShellState> = {
  empty: { kind: "empty" },
  loading: { kind: "loading", displayName: "報告.pdf" },
  "error: corrupted": { kind: "error", code: "corrupted", displayName: "報告.pdf" },
  "error: notPdf": { kind: "error", code: "notPdf", displayName: "notes.pdf" },
  "error: encrypted": { kind: "error", code: "encrypted", displayName: "機密.pdf" },
  "error: workerCrashed": { kind: "error", code: "workerCrashed", displayName: "報告.pdf" },
  open: { kind: "open", document: demoDocument },
  "open: no findings, no outline": {
    kind: "open",
    document: { ...demoDocument, displayName: "乾淨.pdf", outline: [], findings: [] },
  },
};
