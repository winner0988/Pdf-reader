import { describe, expect, it, vi } from "vitest";

import { createPageRenderer, errorCodeOf, type RenderApi } from "@/features/viewer/renderer";
import type { RenderPageArgs } from "@/ipc/generated/contract";
import { createRequestIds } from "@/ipc/requests";

/** A 1 x 1 opaque white raster in the `render_page` wire format. */
function rasterBytes(): ArrayBuffer {
  const bytes = new Uint8Array(16 + 4);
  bytes.set([0x50, 0x44, 0x46, 0x52, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 255, 255, 255, 255]);
  return bytes.buffer;
}

function deferredApi() {
  const pending: { args: RenderPageArgs; resolve: (buffer: ArrayBuffer) => void; reject: (error: unknown) => void }[] = [];
  const api = {
    renderPage: vi.fn(
      (args: RenderPageArgs) =>
        new Promise<ArrayBuffer>((resolve, reject) => pending.push({ args, resolve, reject })),
    ),
    cancel: vi.fn(() => Promise.resolve()),
  } satisfies RenderApi;
  return { api, pending };
}

const page = { doc: 1, pageIndex: 0, scale: 1.5, rotation: "none" as const };

describe("createPageRenderer", () => {
  it("gives every request its own id and decodes the raster", async () => {
    const { api, pending } = deferredApi();
    const renderer = createPageRenderer(api, createRequestIds());

    const first = renderer.render(page);
    renderer.render({ ...page, pageIndex: 1 });
    expect(pending.map((call) => call.args.request)).toEqual([1, 2]);
    expect(pending[1]!.args).toEqual({ ...page, pageIndex: 1, request: 2 });

    pending[0]!.resolve(rasterBytes());
    const raster = await first.result;
    expect([raster.width, raster.height, raster.pixels.length]).toEqual([1, 1, 4]);
  });

  it("cancelling tells the main process and settles as cancelled", async () => {
    const { api, pending } = deferredApi();
    const job = createPageRenderer(api).render(page);

    job.cancel();
    job.cancel();
    await expect(job.result).rejects.toEqual({ code: "cancelled", message: "cancelled" });
    expect(api.cancel).toHaveBeenCalledTimes(1);
    expect(api.cancel).toHaveBeenCalledWith(1);

    // A late answer is ignored.
    pending[0]!.resolve(rasterBytes());
    await Promise.resolve();
  });

  it("does not cancel what already finished", async () => {
    const { api, pending } = deferredApi();
    const job = createPageRenderer(api).render(page);
    pending[0]!.resolve(rasterBytes());
    await job.result;
    job.cancel();
    expect(api.cancel).not.toHaveBeenCalled();
  });

  it("passes main-process errors through and turns bad data into internal errors", async () => {
    const { api, pending } = deferredApi();
    const renderer = createPageRenderer(api);

    const crashed = renderer.render(page);
    pending[0]!.reject({ code: "workerCrashed", message: "" });
    await expect(crashed.result).rejects.toMatchObject({ code: "workerCrashed" });

    const garbage = renderer.render(page);
    pending[1]!.resolve(new ArrayBuffer(3));
    const error = await garbage.result.catch((reason: unknown) => reason);
    expect(errorCodeOf(error)).toBe("internal");
  });
});
