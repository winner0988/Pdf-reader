// Page management (B2-05, ADR 0013), and the edits an earlier run left (B2-13). The page only names
// the document and the edit: the main process checks it, and the document's own worker applies it.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, Edit, UndoArgs } from "@/ipc/generated/contract";

export type EditingApi = {
  /**
   * Applies `edit` to the document `doc`. The tab's new state (a new document id, the pages as
   * they are now, `unsaved`) comes on the open-events channel before this resolves.
   */
  applyEdit(doc: DocumentId, edit: Edit): Promise<void>;
  /**
   * Undoes the document's last edit; its new state comes on the same channel. A document opened
   * with a password needs it again (#94): without one this fails with `encrypted`, and with a
   * wrong one too.
   */
  undo(doc: DocumentId, password?: string): Promise<void>;
  /** Makes the last undone edit again. */
  redo(doc: DocumentId): Promise<void>;
  /**
   * Makes again the edits an earlier run of the app left for the document's file (B2-13); they
   * become its edits, to undo one by one. Refused (`invalidArgument`) while the document has
   * edits of its own.
   */
  recover(doc: DocumentId): Promise<void>;
  /** Discards the edits an earlier run left for the document's file (B2-13). */
  discardRecovered(doc: DocumentId): Promise<void>;
};

export const tauriEditingApi: EditingApi = {
  applyEdit: (doc, edit) => invoke<void>("apply_edit", { args: { doc, edit } }),
  undo: (doc, password) => invoke<void>("undo_edit", { args: { doc, password: password ?? null } satisfies UndoArgs }),
  redo: (doc) => invoke<void>("redo_edit", { doc }),
  recover: (doc) => invoke<void>("recover_edits", { doc }),
  discardRecovered: (doc) => invoke<void>("discard_recovered_edits", { doc }),
};
