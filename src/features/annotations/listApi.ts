import { invoke } from "@tauri-apps/api/core";
import type { AnnotationSummary, DocumentId } from "@/ipc/generated/contract";

export type AnnotationsListApi = {
  getAnnotationSummary(doc: DocumentId): Promise<AnnotationSummary>;
};

export const tauriAnnotationsListApi: AnnotationsListApi = {
  getAnnotationSummary: (doc) => invoke<AnnotationSummary>("get_annotation_summary", { doc }),
};
