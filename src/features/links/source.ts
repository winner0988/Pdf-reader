// Page links from the main process (MVP-12, docs/architecture/ipc-contract.md). Each page is
// asked for once per document; the answer is kept while that document is open.

import { invoke } from "@tauri-apps/api/core";

import type { DocumentId, PageLink } from "@/ipc/generated/contract";

export type LinksApi = {
  getPageLinks(doc: DocumentId, pageIndex: number): Promise<PageLink[]>;
};

export const tauriLinksApi: LinksApi = {
  getPageLinks: (doc, pageIndex) => invoke<PageLink[]>("get_page_links", { doc, pageIndex }),
};

export type LinkSource = {
  links(doc: DocumentId, pageIndex: number): Promise<PageLink[]>;
};

export function createLinkSource(api: LinksApi): LinkSource {
  let cachedDoc: DocumentId | null = null;
  const pages = new Map<number, Promise<PageLink[]>>();
  return {
    links(doc, pageIndex) {
      // One document at a time: another one starts an empty cache.
      if (doc !== cachedDoc) {
        cachedDoc = doc;
        pages.clear();
      }
      let answer = pages.get(pageIndex);
      if (!answer) {
        answer = api.getPageLinks(doc, pageIndex);
        pages.set(pageIndex, answer);
        // A failure is not remembered: the page asks again the next time it is shown.
        answer.catch(() => {
          if (cachedDoc === doc && pages.get(pageIndex) === answer) pages.delete(pageIndex);
        });
      }
      return answer;
    },
  };
}
