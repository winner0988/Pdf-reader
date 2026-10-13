// The annotations of each page, from the main process (B2-07, docs/architecture/annotations.md).
// Each page is asked for once per document: an edit gives the document a new id, and its pages are
// asked again. What arrived is also kept for finding the annotation under the pointer.

import { invoke } from "@tauri-apps/api/core";

import type {
  AnnotationId,
  DocumentId,
  FontId,
  FontInfo,
  PageAnnotation,
  PreparedTextBox,
  StampImageInfo,
  TextBoxInfo,
} from "@/ipc/generated/contract";

export type AnnotationsApi = {
  getPageAnnotations(doc: DocumentId, pageIndex: number): Promise<PageAnnotation[]>;
  /**
   * Asks the user for a picture (PNG or JPEG) in the system's dialog and makes it into a stamp
   * picture of the document (B2-08): only its pixels, kept by the main process, which never
   * tells the path. `null` when the user closes the dialog; rejects with `limitExceeded` (too many
   * pictures in the unsaved changes), `tooLarge` or another code when the picture cannot be used.
   * Without it (demo data, tests) the stamp menu has no picture of the user's own.
   */
  pickStampImage?(doc: DocumentId): Promise<StampImageInfo | null>;
  /**
   * The fonts installed in Windows (B2-27): ids and names, never a path; read again every time, so
   * that a font installed a moment ago is in the list. Without it (demo data, tests) there are no
   * text boxes.
   */
  getFonts?(): Promise<FontInfo[]>;
  /**
   * Makes the words and the font into what a text box is made of (B2-29): the main process reads
   * the font by its id and the document's worker cuts it to the words. Rejects with a code that
   * says why the font or the text cannot be used (`fontMissingGlyphs`, `fontNotTrueType`,
   * `textNotSupported`, ...).
   */
  prepareTextBox?(doc: DocumentId, text: string, font: FontId): Promise<PreparedTextBox>;
  /** What text box `annotation` of a page says of itself, to fill in the dialog that changes it. */
  getTextBox?(doc: DocumentId, pageIndex: number, annotation: AnnotationId): Promise<TextBoxInfo>;
};

export const tauriAnnotationsApi: AnnotationsApi = {
  getPageAnnotations: (doc, pageIndex) => invoke<PageAnnotation[]>("get_page_annotations", { doc, pageIndex }),
  pickStampImage: (doc) => invoke<StampImageInfo | null>("pick_stamp_image", { doc }),
  getFonts: () => invoke<FontInfo[]>("get_fonts"),
  prepareTextBox: (doc, text, font) =>
    invoke<PreparedTextBox>("prepare_text_box", { args: { doc, text, font } }),
  getTextBox: (doc, pageIndex, annotation) =>
    invoke<TextBoxInfo>("get_text_box", { doc, pageIndex, annotation }),
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
