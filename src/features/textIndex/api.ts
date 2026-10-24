// The text index (B3-21, ADR 0026, docs/architecture/text-index.md): the text of the documents
// the user opened, kept encrypted by the main process, to find which document has some words. The
// page gets file names, page numbers and a little text around each hit, never a path.

import { Channel, invoke } from "@tauri-apps/api/core";

import type {
  IndexEntryId,
  IndexSearchArgs,
  IndexSearchEvent,
  OpenIndexedArgs,
  RequestId,
  TabId,
  TextIndexStatus,
} from "@/ipc/generated/contract";

export type TextIndexApi = {
  /** Resolves once the search has finished; rejects if it was cancelled or failed. */
  search(args: IndexSearchArgs, onEvent: (event: IndexSearchEvent) => void): Promise<void>;
  cancel(request: RequestId): Promise<void>;
  /**
   * Opens the document of a result of the last search in a new tab, and answers which. Rejects
   * with `unreadable` if its file is gone or has changed since (it leaves the index).
   */
  open(entry: IndexEntryId): Promise<TabId>;
  status(): Promise<TextIndexStatus>;
  /** Deletes the index. */
  clear(): Promise<void>;
};

export const tauriTextIndexApi: TextIndexApi = {
  search: (args, onEvent) => {
    const channel = new Channel<IndexSearchEvent>();
    channel.onmessage = onEvent;
    return invoke<void>("search_text_index", { args, onEvent: channel });
  },
  cancel: (request) => invoke<void>("cancel", { request }),
  open: (entry) => invoke<TabId>("open_indexed_document", { args: { entry } satisfies OpenIndexedArgs }),
  status: () => invoke<TextIndexStatus>("get_text_index_status"),
  clear: () => invoke<void>("clear_text_index"),
};
