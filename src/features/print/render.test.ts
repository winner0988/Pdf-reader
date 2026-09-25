import { afterEach, describe, expect, it, vi } from "vitest";

import { PRINT_SCALE, PrintCancelled, renderForPrint } from "@/features/print/render";
import type { PageRenderer, RenderJob } from "@/features/viewer/renderer";
import type { IpcError, RenderPageArgs } from "@/ipc/generated/contract";
import type { RasterImage } from "@/ipc/raster";

const LETTER = { widthPt: 612, heightPt: 792 };
const A4_LANDSCAPE = { widthPt: 842, heightPt: 595 };
const sizes = [LETTER, A4_LANDSCAPE, LETTER, LETTER];

type Call = { args: Omit<RenderPageArgs, "request">; resolve: () => void; reject: (error: IpcError) => void; cancel: () => void };

/** Renders on request; the test settles each page. */
function fakeRenderer() {
  const calls: Call[] = [];
  const renderer: PageRenderer = {
    render(args) {
      let resolve = () => {};
      let reject: (error: IpcError) => void = () => {};
      const result = new Promise<RasterImage>((ok, fail) => {
        resolve = () => ok({ width: 1, height: 1, pixels: new Uint8ClampedArray(4) });
        reject = fail;
      });
      const cancel = vi.fn(() => reject({ code: "cancelled", message: "" }));
      calls.push({ args, resolve, reject, cancel });
      return { result, cancel } satisfies RenderJob;
    },
  };
  return { renderer, calls };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
let urls = 0;
const encode = vi.fn(async () => `blob:page-${++urls}`);

afterEach(() => {
  vi.restoreAllMocks();
  encode.mockClear();
});

describe("renderForPrint", () => {
  it("renders the pages in order at print resolution, unturned, one at a time", async () => {
    const { renderer, calls } = fakeRenderer();
    const onProgress = vi.fn();
    const printing = renderForPrint({ renderer, doc: 4, pages: [1, 3], sizes, onProgress, signal: new AbortController().signal, encode });
    await settle();
    // One page at a time: the second is asked for only once the first is done.
    expect(calls).toHaveLength(1);
    expect(calls[0]!.args).toEqual({ doc: 4, pageIndex: 1, scale: PRINT_SCALE, rotation: "none" });
    calls[0]!.resolve();
    await settle();
    expect(onProgress).toHaveBeenLastCalledWith(1);
    calls[1]!.resolve();
    const pages = await printing;
    expect(pages.map((page) => [page.pageIndex, page.size])).toEqual([
      [1, A4_LANDSCAPE],
      [3, LETTER],
    ]);
    expect(onProgress).toHaveBeenLastCalledWith(2);
  });

  it("stops when cancelled, and frees the pages it made", async () => {
    const revoke = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    const { renderer, calls } = fakeRenderer();
    const controller = new AbortController();
    const printing = renderForPrint({ renderer, doc: 4, pages: [0, 1, 2], sizes, onProgress: () => {}, signal: controller.signal, encode });
    await settle();
    calls[0]!.resolve();
    await settle();
    controller.abort();
    await expect(printing).rejects.toBeInstanceOf(PrintCancelled);
    expect(calls[1]!.cancel).toHaveBeenCalled();
    expect(calls).toHaveLength(2);
    expect(revoke).toHaveBeenCalledTimes(1);
  });

  it("gives up on a page that cannot be rendered", async () => {
    vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    const { renderer, calls } = fakeRenderer();
    const printing = renderForPrint({ renderer, doc: 4, pages: [0, 1], sizes, onProgress: () => {}, signal: new AbortController().signal, encode });
    await settle();
    calls[0]!.reject({ code: "workerCrashed", message: "" });
    await expect(printing).rejects.toMatchObject({ code: "workerCrashed" });
    expect(calls).toHaveLength(1);
  });
});
