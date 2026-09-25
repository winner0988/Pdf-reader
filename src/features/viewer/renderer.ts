// Page render requests from the viewer to the main process (MVP-07, docs/architecture/rendering.md).

import { invoke } from "@tauri-apps/api/core";

import type { ErrorCode, IpcError, RenderPageArgs, RequestId } from "@/ipc/generated/contract";
import { decodeRaster, type RasterImage } from "@/ipc/raster";
import { nextRequestId } from "@/ipc/requests";

/** The main-process commands the viewer needs. */
export type RenderApi = {
  renderPage(args: RenderPageArgs): Promise<ArrayBuffer>;
  cancel(request: RequestId): Promise<void>;
};

export const tauriRenderApi: RenderApi = {
  renderPage: (args) => invoke<ArrayBuffer>("render_page", { args }),
  cancel: (request) => invoke<void>("cancel", { request }),
};

export type RenderJob = {
  /** Resolves with the page, or rejects with an `IpcError`-shaped value. */
  result: Promise<RasterImage>;
  /** Stops waiting; a queued request is dropped by the main process. Idempotent. */
  cancel(): void;
};

export type PageRenderer = {
  render(args: Omit<RenderPageArgs, "request">): RenderJob;
};

const cancelled: IpcError = { code: "cancelled", message: "cancelled" };

export function errorCodeOf(error: unknown): ErrorCode {
  const code = (error as Partial<IpcError> | null)?.code;
  return typeof code === "string" ? code : "internal";
}

/** Draws a raster into the canvas at its native resolution; CSS scales it to its box. */
export function drawRaster(canvas: HTMLCanvasElement | null, raster: RasterImage) {
  const context = canvas?.getContext("2d");
  if (!canvas || !context) return;
  canvas.width = raster.width;
  canvas.height = raster.height;
  context.putImageData(new ImageData(raster.pixels, raster.width, raster.height), 0, 0);
}

export function createPageRenderer(api: RenderApi, nextId: () => RequestId = nextRequestId): PageRenderer {
  return {
    render(args) {
      const request = nextId();
      let settled = false;
      let rejectEarly: (reason: IpcError) => void = () => {};

      const response = api.renderPage({ ...args, request }).then(decodeRaster);
      const result = new Promise<RasterImage>((resolve, reject) => {
        rejectEarly = reject;
        response.then(
          (raster) => {
            settled = true;
            resolve(raster);
          },
          (error: unknown) => {
            settled = true;
            reject(error instanceof Error ? { code: "internal", message: error.message } : error);
          },
        );
      });

      return {
        result,
        cancel() {
          if (settled) return;
          settled = true;
          rejectEarly(cancelled);
          // Best effort: only helps while the request is still queued.
          api.cancel(request).catch(() => {});
        },
      };
    },
  };
}
