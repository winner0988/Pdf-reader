// Main-process commands for document properties and font discovery (B5-04).

import { invoke } from "@tauri-apps/api/core";

import type { DocumentFonts, DocumentId, DocumentProperties, RequestId } from "@/ipc/generated/contract";

export type PropertiesApi = {
  /** Retrieves basic metadata, format, encryption, signature and attachment counts. */
  getDocumentProperties(doc: DocumentId): Promise<DocumentProperties>;
  /** Discovers fonts embedded or referenced in the document. */
  getDocumentFonts(doc: DocumentId, request: RequestId): Promise<DocumentFonts>;
  /** Cancels an ongoing font discovery request. */
  cancel(request: RequestId): Promise<void>;
};

export const defaultPropertiesApi: PropertiesApi = {
  getDocumentProperties: (doc) => invoke("get_document_properties", { doc }),
  getDocumentFonts: (doc, request) => invoke("get_document_fonts", { doc, request }),
  cancel: (request) => invoke("cancel", { request }),
};
