// The signatures of the open document, verified offline by its worker (B2-14, ADR 0014,
// docs/architecture/signatures.md).

import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import type { DocumentId, SignatureReport } from "@/ipc/generated/contract";

export type SignaturesApi = {
  getSignatures(doc: DocumentId): Promise<SignatureReport>;
};

export const tauriSignaturesApi: SignaturesApi = {
  getSignatures: (doc) => invoke<SignatureReport>("get_signatures", { doc }),
};

/** What the reader shows of a document's signatures. */
export type SignatureView = { status: "none" } | { status: "ready"; report: SignatureReport };

/**
 * Verifies the signatures of `doc`, which are those of its file: so not while it has changes of
 * its own (the last answer stays), and again when saving gives the file new content. A document
 * without signatures, one whose check failed and one not checked yet all show nothing; answers
 * for other documents are ignored, and another file starts afresh (`session`).
 */
export function useSignatures(
  api: SignaturesApi,
  doc: DocumentId | null,
  session: number | undefined,
  unsaved: boolean,
): SignatureView {
  const [loaded, setLoaded] = useState<{ session: number | undefined; report: SignatureReport } | null>(null);

  useEffect(() => {
    if (doc === null || unsaved) return;
    let current = true;
    api.getSignatures(doc).then(
      (report) => {
        if (current) setLoaded({ session, report });
      },
      () => {
        if (current) setLoaded(null);
      },
    );
    return () => {
      current = false;
    };
  }, [api, doc, session, unsaved]);

  if (doc === null || loaded === null || loaded.session !== session || loaded.report.signatures.length === 0) {
    return { status: "none" };
  }
  return { status: "ready", report: loaded.report };
}
