import { useEffect, useState } from "react";
import type { AttachmentInfo, DocumentId } from "@/ipc/generated/contract";
import type { AttachmentsApi } from "./api";

export type AttachmentsState =
  | { status: "loading" }
  | { status: "failed" }
  | { status: "ready"; items: AttachmentInfo[] };

/** Loads the attachments of `doc`; answers for other documents are ignored. */
export function useAttachments(api: AttachmentsApi, doc: DocumentId | undefined): AttachmentsState {
  const [loaded, setLoaded] = useState<{ doc: DocumentId; state: AttachmentsState } | null>(null);

  useEffect(() => {
    if (doc === undefined) return;
    let current = true;
    api.getAttachments(doc).then(
      (items) => {
        if (current) setLoaded({ doc, state: { status: "ready", items } });
      },
      () => {
        if (current) setLoaded({ doc, state: { status: "failed" } });
      },
    );
    return () => {
      current = false;
    };
  }, [api, doc]);

  if (doc === undefined) return { status: "ready", items: [] };
  return loaded?.doc === doc ? loaded.state : { status: "loading" };
}
