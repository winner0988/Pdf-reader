// Exporting pages as text or PNG files (B2-04, docs/architecture/export.md). The page says what
// to export; the main process asks the user where, in the system's dialogs, and writes the files.
// No file content or path passes through the page.

import { Channel, invoke } from "@tauri-apps/api/core";

import type { DocumentId, ExportArgs, ExportEvent, ExportFormat, RequestId } from "@/ipc/generated/contract";
import { nextRequestId } from "@/ipc/requests";

export type ExportJob = {
  request: RequestId;
  /** True once the files are written; false if the user closed a dialog; rejects on failure. */
  done: Promise<boolean>;
};

export type ExportApi = {
  exportPages(
    doc: DocumentId,
    pages: number[],
    format: ExportFormat,
    onProgress: (pagesDone: number, total: number) => void,
  ): ExportJob;
  /** Stops an export before its next page; the pages already written stay. */
  cancel(request: RequestId): Promise<void>;
};

export const tauriExportApi: ExportApi = {
  exportPages(doc, pages, format, onProgress) {
    const request = nextRequestId();
    const channel = new Channel<ExportEvent>();
    channel.onmessage = (event) => onProgress(event.pagesDone, event.total);
    const args: ExportArgs = { request, doc, pages, format };
    return { request, done: invoke<boolean>("export_pages", { args, onEvent: channel }) };
  },
  cancel: (request) => invoke<void>("cancel", { request }),
};
