import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import type { OutlineView } from "@/features/outline/tree";
import type { DocumentId, OutlineResult } from "@/ipc/generated/contract";

export type OutlineApi = {
  getOutline(doc: DocumentId): Promise<OutlineResult>;
};

export const tauriOutlineApi: OutlineApi = {
  getOutline: (doc) => invoke<OutlineResult>("get_outline", { doc }),
};

/** Loads the outline of `doc` when it has one; answers for other documents are ignored. */
export function useOutline(api: OutlineApi, doc: DocumentId | null, hasOutline: boolean): OutlineView {
  const [loaded, setLoaded] = useState<{ doc: DocumentId; view: OutlineView } | null>(null);

  useEffect(() => {
    if (doc === null || !hasOutline) return;
    let current = true;
    api.getOutline(doc).then(
      (outline) => {
        if (current) setLoaded({ doc, view: { status: "ready", items: outline.items, truncated: outline.truncated } });
      },
      () => {
        if (current) setLoaded({ doc, view: { status: "failed" } });
      },
    );
    return () => {
      current = false;
    };
  }, [api, doc, hasOutline]);

  if (doc === null || !hasOutline) return { status: "none" };
  return loaded?.doc === doc ? loaded.view : { status: "loading" };
}
