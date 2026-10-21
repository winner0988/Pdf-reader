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

/**
 * Loads the outline of `doc` when it has one; answers for other documents are ignored. An edit
 * gives the document a new `doc` (B2-02): while the outline after it loads, the one before stays
 * shown for the same file (`session`), so that the tree keeps its place and its focus (B3-17).
 */
export function useOutline(
  api: OutlineApi,
  doc: DocumentId | null,
  hasOutline: boolean,
  session?: number,
): OutlineView {
  const [loaded, setLoaded] = useState<{ doc: DocumentId; session?: number; view: OutlineView } | null>(null);

  useEffect(() => {
    if (doc === null || !hasOutline) return;
    let current = true;
    api.getOutline(doc).then(
      (outline) => {
        if (current) {
          setLoaded({ doc, session, view: { status: "ready", items: outline.items, truncated: outline.truncated } });
        }
      },
      () => {
        if (current) setLoaded({ doc, session, view: { status: "failed" } });
      },
    );
    return () => {
      current = false;
    };
  }, [api, doc, hasOutline, session]);

  if (doc === null || !hasOutline) return { status: "none" };
  if (loaded?.doc === doc) return loaded.view;
  if (session !== undefined && loaded?.session === session && loaded.view.status === "ready") return loaded.view;
  return { status: "loading" };
}
