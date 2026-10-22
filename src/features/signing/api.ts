// Signing a document (B3-06, ADR 0020, docs/architecture/signing.md). The page never sees a path,
// a certificate or a key: the main process asks for the certificate file in its own dialog and
// keeps it, gets the password once, signs, asks where the signed copy goes, and opens it.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, SignArgs, SignCheck, SignResult, SigningFile } from "@/ipc/generated/contract";

export type SigningApi = {
  /** Whether the document can be signed, or the first reason it cannot. */
  check(doc: DocumentId): Promise<SignCheck>;
  /** Asks for the certificate file in the system's open dialog: `null` if it was closed. */
  chooseFile(): Promise<SigningFile | null>;
  /**
   * Signs with the certificate file chosen last and writes the signed copy where the user says in
   * the system's save dialog, never over the document's own file; the copy opens in a tab of its
   * own. `null` if the dialog was closed. The password goes to the main process once and is not
   * kept.
   */
  sign(args: SignArgs): Promise<SignResult | null>;
};

export const tauriSigningApi: SigningApi = {
  check: (doc) => invoke<SignCheck>("check_signing", { doc }),
  chooseFile: () => invoke<SigningFile | null>("choose_signing_file"),
  sign: (args) => invoke<SignResult | null>("sign_document", { args }),
};
