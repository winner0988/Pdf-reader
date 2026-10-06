// The annotations of each page, from the main process (B2-07, docs/architecture/annotations.md).
// Each page is asked for once per document: an edit gives the document a new id, and its pages are
// asked again. What arrived is also kept for finding the annotation under the pointer.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, PageAnnotation } from "@/ipc/generated/contract";

export type AnnotationsApi = {
  getPageAnnotations(doc: DocumentId, pageIndex: number): Promise<PageAnnotation[]>;
};

export const tauriAnnotationsApi: AnnotationsApi = {
  getPageAnnotations: (doc, pageIndex) => invoke<PageAnnotation[]>("get_page_annotations", { doc, pageIndex }),
};

export type AnnotationSource = {
  annotations(doc: DocumentId, pageIndex: number): Promise<PageAnnotation[]>;
  /** What arrived for the page, if anything has. */
  loaded(doc: DocumentId, pageIndex: number): PageAnnotation[] | undefined;
};

export function createAnnotationSource(api: AnnotationsApi): AnnotationSource {
  let cachedDoc: DocumentId | null = null;
  const pages = new Map<number, Promise<PageAnnotation[]>>();
  const arrived = new Map<number, PageAnnotation[]>();
  const forOne = (doc: DocumentId) => {
    // One document at a time: another one starts an empty cache.
    if (doc !== cachedDoc) {
      cachedDoc = doc;
      pages.clear();
      arrived.clear();
    }
  };
  return {
    annotations(doc, pageIndex) {
      forOne(doc);
      let answer = pages.get(pageIndex);
      if (!answer) {
        const asked = api.getPageAnnotations(doc, pageIndex);
        answer = asked;
        pages.set(pageIndex, asked);
        asked.then(
          (annotations) => {
            if (cachedDoc === doc && pages.get(pageIndex) === asked) arrived.set(pageIndex, annotations);
          },
          // A failure is not remembered: the page asks again the next time it is shown.
          () => {
            if (cachedDoc === doc && pages.get(pageIndex) === asked) pages.delete(pageIndex);
          },
        );
      }
      return answer;
    },
    loaded(doc, pageIndex) {
      return doc === cachedDoc ? arrived.get(pageIndex) : undefined;
    },
  };
}
