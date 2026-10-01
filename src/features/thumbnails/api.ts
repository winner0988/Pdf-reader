// Page management (B2-05, ADR 0013). The page only names the document and the edit: the main
// process checks it, and the document's own worker applies it.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, Edit } from "@/ipc/generated/contract";

export type EditingApi = {
  /**
   * Applies `edit` to the document `doc`. The tab's new state (a new document id, the pages as
   * they are now, `unsaved`) comes on the open-events channel before this resolves.
   */
  applyEdit(doc: DocumentId, edit: Edit): Promise<void>;
  /** Undoes the document's last edit; its new state comes on the same channel. */
  undo(doc: DocumentId): Promise<void>;
  /** Makes the last undone edit again. */
  redo(doc: DocumentId): Promise<void>;
};

export const tauriEditingApi: EditingApi = {
  applyEdit: (doc, edit) => invoke<void>("apply_edit", { args: { doc, edit } }),
  undo: (doc) => invoke<void>("undo_edit", { doc }),
  redo: (doc) => invoke<void>("redo_edit", { doc }),
};
