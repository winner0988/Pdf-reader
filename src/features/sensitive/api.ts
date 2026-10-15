// Sensitive documents (B3-05, ADR 0004, ADR 0010, docs/architecture/sensitive-files.md): the mark
// in the file, the list of files marked on this computer, and the clipboard that keeps their text
// out of Windows' clipboard history.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, MarkCheck, SensitiveArgs, SensitiveFile, SensitiveId } from "@/ipc/generated/contract";

export type SensitiveApi = {
  /** What writing the mark into the open document's file, or taking it out, would do. */
  check(doc: DocumentId): Promise<MarkCheck>;
  /**
   * Writes the mark into the file, or takes it out, once the user confirmed; `password` is the
   * permissions password of an encrypted file. The tab's new state arrives on the open-events
   * channel.
   */
  set(doc: DocumentId, sensitive: boolean, password: string | null): Promise<void>;
  /** The files marked on this computer, most recently marked first. */
  list(): Promise<SensitiveFile[]>;
  /** Takes a file off that list and returns the list; the file stays marked. */
  remove(id: SensitiveId): Promise<SensitiveFile[]>;
  /** Puts text copied from a sensitive document on the clipboard, out of its history and the cloud. */
  copy(text: string): Promise<void>;
};

export const tauriSensitiveApi: SensitiveApi = {
  check: (doc) => invoke<MarkCheck>("check_sensitive_mark", { doc }),
  set: (doc, sensitive, password) =>
    invoke<void>("set_sensitive", { args: { doc, sensitive, password } satisfies SensitiveArgs }),
  list: () => invoke<SensitiveFile[]>("get_sensitive_files"),
  remove: (id) => invoke<SensitiveFile[]>("remove_sensitive_file", { id }),
  copy: (text) => invoke<void>("copy_protected_text", { text }),
};

/** Whether `text` fits what one protected copy takes (`LIMITS.maxClipboardTextBytes`, in UTF-8). */
export function fitsClipboard(text: string, maxBytes: number): boolean {
  // Most text is far below the limit: only count the bytes when it could be over.
  return text.length * 3 <= maxBytes || new TextEncoder().encode(text).length <= maxBytes;
}
