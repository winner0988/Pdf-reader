// Text comparison between two open tabs (B3-24, E9, docs/architecture/compare.md).
// Diff computation and export generation happen directly in the main process.
// No document text passes through the page.

import { invoke } from "@tauri-apps/api/core";

import type {
  CompareTextArgs,
  CompareTextResult,
  ExportCompareTextArgs,
  ExportCompareTextOutcome,
} from "@/ipc/generated/contract";

export type CompareApi = {
  compareText(args: CompareTextArgs): Promise<CompareTextResult>;
  exportCompareText(args: ExportCompareTextArgs): Promise<ExportCompareTextOutcome>;
};

export const tauriCompareApi: CompareApi = {
  compareText: (args) => invoke<CompareTextResult>("compare_text", { args }),
  exportCompareText: (args) => invoke<ExportCompareTextOutcome>("export_compare_text", { args }),
};
