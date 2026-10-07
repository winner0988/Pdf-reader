// Page management (B2-05, ADR 0013), and the edits an earlier run left (B2-13). The page only names
// the document and the edit: the main process checks it, and the document's own worker applies it.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, Edit, PagesSource, UndoArgs, UnlockSourceArgs } from "@/ipc/generated/contract";

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
  /**
   * Asks for a PDF whose pages go into the document (B2-06): the main process asks in the
   * system's dialog and keeps a clean copy of the file; the path never comes here. `null` when
   * the user closes the dialog; rejects with `encrypted` for a file that needs a password (then
   * `unlockPagesSource`), `notAllowed` for one whose author forbids taking pages out, and
   * `limitExceeded`, `tooLarge` or others for a file that cannot be used. Without it (demo data)
   * pages cannot be taken from a file.
   */
  pickPagesSource?(doc: DocumentId): Promise<PagesSource | null>;
  /** Tries `password` on the file `pickPagesSource` could not open without one. */
  unlockPagesSource?(doc: DocumentId, password: string): Promise<PagesSource>;
};

export const tauriEditingApi: EditingApi = {
  applyEdit: (doc, edit) => invoke<void>("apply_edit", { args: { doc, edit } }),
  undo: (doc, password) => invoke<void>("undo_edit", { args: { doc, password: password ?? null } satisfies UndoArgs }),
  redo: (doc) => invoke<void>("redo_edit", { doc }),
  recover: (doc) => invoke<void>("recover_edits", { doc }),
  discardRecovered: (doc) => invoke<void>("discard_recovered_edits", { doc }),
  pickPagesSource: (doc) => invoke<PagesSource | null>("pick_pages_source", { doc }),
  unlockPagesSource: (doc, password) =>
    invoke<PagesSource>("unlock_pages_source", { args: { doc, password } satisfies UnlockSourceArgs }),
};
