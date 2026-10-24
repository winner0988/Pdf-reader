// Signing a document (B3-06, ADR 0020, docs/architecture/signing.md). The page never sees a path,
// a certificate or a key: the main process lists the certificates of the user's own store, or asks
// for a certificate file in its own dialog and keeps it, gets the file's password once, signs, asks
// where the signed copy goes, and opens it. A card's PIN is asked for by Windows or the card.

import { invoke } from "@tauri-apps/api/core";

import type {
  DocumentId,
  SignArgs,
  SignCheck,
  SignResult,
  SigningFile,
  StoreCertificate,
} from "@/ipc/generated/contract";

export type SigningApi = {
  /** Whether the document can be signed, or the first reason it cannot. */
  check(doc: DocumentId): Promise<SignCheck>;
  /** Asks for the certificate file in the system's open dialog: `null` if it was closed. */
  chooseFile(): Promise<SigningFile | null>;
  /**
   * The certificates of the user's own store with a private key (on a smart card, too), those that
   * can sign first; their numbers replace those of the list before.
   */
  listCertificates(): Promise<StoreCertificate[]>;
  /**
   * Signs with the certificate file chosen last or a certificate of the list given last, and
   * writes the signed copy where the user says in the system's save dialog, never over the
   * document's own file; the copy opens in a tab of its own. `null` if the dialog was closed. A
   * file's password goes to the main process once and is not kept.
   */
  sign(args: SignArgs): Promise<SignResult | null>;
};

export const tauriSigningApi: SigningApi = {
  check: (doc) => invoke<SignCheck>("check_signing", { doc }),
  chooseFile: () => invoke<SigningFile | null>("choose_signing_file"),
  listCertificates: () => invoke<StoreCertificate[]>("list_signing_certificates"),
  sign: (args) => invoke<SignResult | null>("sign_document", { args }),
};
