import { useEffect, useState } from "react";
import type { AnnotationSummary, DocumentId } from "@/ipc/generated/contract";
import { type AnnotationsListApi, tauriAnnotationsListApi } from "./listApi";

export type AnnotationSummaryState =
  | { status: "idle" }
  | { status: "loading"; doc: DocumentId; summary?: AnnotationSummary }
  | { status: "ready"; doc: DocumentId; summary: AnnotationSummary }
  | { status: "failed"; doc: DocumentId };

/**
 * Fetches the document's annotation summary when the panel is active (B4-06).
 * Debounces requests by 300 ms across edits and discards responses that do not match
 * the current document id.
 */
export function useAnnotationSummary(
  doc: DocumentId | undefined,
  active: boolean,
  api: AnnotationsListApi = tauriAnnotationsListApi,
  delayMs = 300,
) {
  const [state, setState] = useState<AnnotationSummaryState>({ status: "idle" });

  useEffect(() => {
    if (!active || doc === undefined) {
      return;
    }

    let cancelled = false;

    const timer = setTimeout(() => {
      setState((prev) => ({
        status: "loading",
        doc,
        summary: prev.status === "ready" ? prev.summary : undefined,
      }));

      api.getAnnotationSummary(doc).then(
        (summary) => {
          if (!cancelled) {
            setState({ status: "ready", doc, summary });
          }
        },
        () => {
          if (!cancelled) {
            setState({ status: "failed", doc });
          }
        },
      );
    }, delayMs);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [doc, active, api, delayMs]);

  return state;
}
