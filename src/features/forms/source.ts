// The form fields of each page, from the main process (B2-09, docs/architecture/forms.md). Each
// page is asked for once per document: an edit gives the document a new id, and its pages are
// asked again.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, FormField } from "@/ipc/generated/contract";

export type FormsApi = {
  getPageFields(doc: DocumentId, pageIndex: number): Promise<FormField[]>;
};

export const tauriFormsApi: FormsApi = {
  getPageFields: (doc, pageIndex) => invoke<FormField[]>("get_page_fields", { doc, pageIndex }),
};

export type FormSource = {
  fields(doc: DocumentId, pageIndex: number): Promise<FormField[]>;
};

export function createFormSource(api: FormsApi): FormSource {
  let cachedDoc: DocumentId | null = null;
  const pages = new Map<number, Promise<FormField[]>>();
  return {
    fields(doc, pageIndex) {
      // One document at a time: another one starts an empty cache.
      if (doc !== cachedDoc) {
        cachedDoc = doc;
        pages.clear();
      }
      let answer = pages.get(pageIndex);
      if (!answer) {
        const asked = api.getPageFields(doc, pageIndex);
        answer = asked;
        pages.set(pageIndex, asked);
        // A failure is not remembered: the page asks again the next time it is shown.
        asked.catch(() => {
          if (cachedDoc === doc && pages.get(pageIndex) === asked) pages.delete(pageIndex);
        });
      }
      return answer;
    },
  };
}
