// Saving documents (B2-02, ADR 0013). The page only says which document: the main process knows
// the file, shows the save dialog and writes it; no path or file content passes through the page.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, EncryptArgs, SaveResult } from "@/ipc/generated/contract";

export type SavingApi = {
  /** Writes the document, with its changes, to its own file. */
  save(doc: DocumentId): Promise<SaveResult>;
  /** Asks where in the system's save dialog, then writes there: `null` if the dialog was closed. */
  saveAs(doc: DocumentId): Promise<SaveResult | null>;
  /** Closes the window once the user was asked about unsaved changes; `discard` drops them. */
  closeWindow(discard: boolean): Promise<void>;
  /**
   * Writes a copy of the document without its metadata (B2-03) where the user says in the
   * system's save dialog, never over its own file: `false` if the dialog was closed. The
   * document itself does not change.
   */
  privacyExport(doc: DocumentId): Promise<boolean>;
  /**
   * Writes a copy of the document encrypted with AES-256 (B2-15) where the user says in the
   * system's save dialog, never over its own file: `false` if the dialog was closed. The
   * document itself does not change. The passwords go to the main process once and are not kept.
   */
  encryptCopy(args: EncryptArgs): Promise<boolean>;
};

export const tauriSavingApi: SavingApi = {
  save: (doc) => invoke<SaveResult>("save_document", { doc }),
  saveAs: (doc) => invoke<SaveResult | null>("save_document_as", { doc }),
  closeWindow: (discard) => invoke<void>("close_window", { discard }),
  privacyExport: (doc) => invoke<boolean>("privacy_export", { doc }),
  encryptCopy: (args) => invoke<boolean>("encrypt_copy", { args }),
};
