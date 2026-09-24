// Renders pages for printing (MVP-17, docs/architecture/printing.md): at print resolution, without
// the view's zoom and rotation, one page at a time. Each page becomes a PNG in this process's
// memory (a blob URL) until printing is over.

import type { PageSize } from "@/features/shell/model";
import { errorCodeOf, type PageRenderer, type RenderJob } from "@/features/viewer/renderer";
import type { DocumentId } from "@/ipc/generated/contract";
import type { RasterImage } from "@/ipc/raster";

/** Dots per inch pages are printed at: sharp text, a few megabytes per page while rendering. */
export const PRINT_DPI = 200;
/** The worker's scale for it: device pixels per PDF point (72 per inch). */
export const PRINT_SCALE = PRINT_DPI / 72;

export type PrintPage = { pageIndex: number; url: string; size: PageSize };

/** A rendered page as a PNG blob URL. */
export type Encode = (raster: RasterImage) => Promise<string>;

export const encodePng: Encode = (raster) =>
  new Promise((resolve, reject) => {
    const canvas = document.createElement("canvas");
    canvas.width = raster.width;
    canvas.height = raster.height;
    const context = canvas.getContext("2d");
    if (!context) {
      reject(new Error("no 2D canvas"));
      return;
    }
    context.putImageData(new ImageData(raster.pixels, raster.width, raster.height), 0, 0);
    canvas.toBlob((blob) => (blob ? resolve(URL.createObjectURL(blob)) : reject(new Error("no PNG"))), "image/png");
  });

type Options = {
  renderer: PageRenderer;
  doc: DocumentId;
  pages: readonly number[];
  sizes: readonly PageSize[];
  onProgress: (done: number) => void;
  signal: AbortSignal;
  encode?: Encode;
};

export class PrintCancelled extends Error {}

/**
 * Renders `pages` (0-based, in order) for printing. Stops and frees what it made when `signal`
 * aborts (rejecting with `PrintCancelled`) or a page fails (rejecting with its error code).
 */
export async function renderForPrint({
  renderer,
  doc,
  pages,
  sizes,
  onProgress,
  signal,
  encode = encodePng,
}: Options): Promise<PrintPage[]> {
  const done: PrintPage[] = [];
  let job: RenderJob | null = null;
  const abort = () => job?.cancel();
  signal.addEventListener("abort", abort);
  try {
    for (const pageIndex of pages) {
      if (signal.aborted) throw new PrintCancelled();
      job = renderer.render({ doc, pageIndex, scale: PRINT_SCALE, rotation: "none" });
      let raster: RasterImage;
      try {
        raster = await job.result;
      } catch (error) {
        if (signal.aborted || errorCodeOf(error) === "cancelled") throw new PrintCancelled();
        throw error;
      }
      const url = await encode(raster);
      done.push({ pageIndex, url, size: sizes[pageIndex]! });
      onProgress(done.length);
    }
    if (signal.aborted) throw new PrintCancelled();
    return done;
  } catch (error) {
    freePrintPages(done);
    throw error;
  } finally {
    signal.removeEventListener("abort", abort);
  }
}

export function freePrintPages(pages: readonly PrintPage[]) {
  for (const page of pages) URL.revokeObjectURL(page.url);
}
