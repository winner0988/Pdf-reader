// Page links from the main process (MVP-12, docs/architecture/ipc-contract.md). Each page is
// asked for once per document; the answer is kept while that document is open.

import { invoke } from "@tauri-apps/api/core";

import type {
  DocumentId,
  LinkArgs,
  LinkId,
  LinkPreview,
  OutlineLinkArgs,
  PageLink,
} from "@/ipc/generated/contract";

export type LinksApi = {
  getPageLinks(doc: DocumentId, pageIndex: number): Promise<PageLink[]>;
  /** What the confirmation shows about a web link. */
  describeLink(doc: DocumentId, link: LinkId): Promise<LinkPreview>;
  /**
   * Opens a web link the user confirmed. It is named by id only: the main process gets the URI
   * from the worker again and checks it; no URI ever goes from here to the system.
   */
  openLink(doc: DocumentId, link: LinkId): Promise<void>;
  /** The same for the web link of an outline item, named by its position in the outline (#49). */
  describeOutlineLink(doc: DocumentId, item: number): Promise<LinkPreview>;
  openOutlineLink(doc: DocumentId, item: number): Promise<void>;
};

export const tauriLinksApi: LinksApi = {
  getPageLinks: (doc, pageIndex) => invoke<PageLink[]>("get_page_links", { doc, pageIndex }),
  describeLink: (doc, link) => invoke<LinkPreview>("describe_link", { args: { doc, link } satisfies LinkArgs }),
  openLink: (doc, link) => invoke<void>("open_link", { args: { doc, link } satisfies LinkArgs }),
  describeOutlineLink: (doc, item) =>
    invoke<LinkPreview>("describe_outline_link", { args: { doc, item } satisfies OutlineLinkArgs }),
  openOutlineLink: (doc, item) => invoke<void>("open_outline_link", { args: { doc, item } satisfies OutlineLinkArgs }),
};

export type LinkSource = {
  links(doc: DocumentId, pageIndex: number): Promise<PageLink[]>;
};

export function createLinkSource(api: Pick<LinksApi, "getPageLinks">): LinkSource {
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
