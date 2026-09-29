// The recently opened files (#73, docs/architecture/recent-files.md). The main process keeps the
// paths, in the app's local data folder; the page gets file names and ids only.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, FileRecordingArgs, RecentFile, RecentId } from "@/ipc/generated/contract";

export type RecentApi = {
  /** Most recent first. */
  list(): Promise<RecentFile[]>;
  /** Opens a listed file in a new tab; rejects with `unreadable` (and drops it) if it is gone. */
  open(id: RecentId): Promise<void>;
  /** Takes a file off the list and returns the list. */
  remove(id: RecentId): Promise<RecentFile[]>;
  clear(): Promise<void>;
  /** Forgets which files the user asked not to record (the settings, B2-12). */
  clearExclusions(): Promise<void>;
  /** Whether an open document's file may be on the list ("不記錄此檔案" unchecked). */
  isRecorded(doc: DocumentId): Promise<boolean>;
  setRecorded(doc: DocumentId, record: boolean): Promise<void>;
};

export const tauriRecentApi: RecentApi = {
  list: () => invoke<RecentFile[]>("get_recent_files"),
  open: (id) => invoke<void>("open_recent_file", { id }),
  remove: (id) => invoke<RecentFile[]>("remove_recent_file", { id }),
  clear: () => invoke<void>("clear_recent_files"),
  clearExclusions: () => invoke<void>("clear_recent_exclusions"),
  isRecorded: (doc) => invoke<boolean>("get_file_recording", { doc }),
  setRecorded: (doc, record) =>
    invoke<void>("set_file_recording", { args: { doc, record } satisfies FileRecordingArgs }),
};
