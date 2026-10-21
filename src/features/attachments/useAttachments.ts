import { useEffect, useState } from "react";
import type { AttachmentInfo, DocumentId } from "@/ipc/generated/contract";
import type { AttachmentsApi } from "./api";

export type AttachmentsState =
  | { status: "loading" }
  | { status: "failed" }
  | { status: "ready"; items: AttachmentInfo[] };

export function useAttachments(api: AttachmentsApi, doc: DocumentId | undefined): AttachmentsState {
  const [state, setState] = useState<AttachmentsState>({ status: "loading" });

  useEffect(() => {
    if (doc === undefined) {
      setState({ status: "ready", items: [] });
      return;
    }
    let current = true;
    setState({ status: "loading" });
    api.getAttachments(doc).then(
      (items) => {
        if (current) setState({ status: "ready", items });
      },
      () => {
        if (current) setState({ status: "failed" });
      },
    );
    return () => {
      current = false;
    };
  }, [api, doc]);

  return state;
}
