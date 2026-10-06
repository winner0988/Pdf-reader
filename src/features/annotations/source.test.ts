import { describe, expect, it, vi } from "vitest";

import { createAnnotationSource, type AnnotationsApi } from "@/features/annotations/source";
import type { PageAnnotation } from "@/ipc/generated/contract";

const annotation = (id: number): PageAnnotation => ({
  id,
  kind: "note",
  rect: { x0: 0, y0: 0, x1: 20, y1: 20 },
  color: null,
  text: `note ${id}`,
});

describe("the annotations of each page", () => {
  it("are asked for once per page and document, and kept for finding one by position", async () => {
    const api = {
      getPageAnnotations: vi.fn((_doc: number, page: number) => Promise.resolve([annotation(page + 10)])),
    } satisfies AnnotationsApi;
    const source = createAnnotationSource(api);
    expect(source.loaded(1, 0)).toBeUndefined();
    const first = source.annotations(1, 0);
    expect(source.annotations(1, 0)).toBe(first);
    expect(await first).toEqual([annotation(10)]);
    expect(source.loaded(1, 0)).toEqual([annotation(10)]);
    expect(api.getPageAnnotations).toHaveBeenCalledTimes(1);

    // An edit gives the document a new id: its pages are asked again, and the old ones forgotten.
    await source.annotations(2, 0);
    expect(api.getPageAnnotations).toHaveBeenCalledTimes(2);
    expect(source.loaded(1, 0)).toBeUndefined();
    expect(source.loaded(2, 0)).toEqual([annotation(10)]);
  });

  it("are asked for again after a failure", async () => {
    const api = {
      getPageAnnotations: vi
        .fn<AnnotationsApi["getPageAnnotations"]>()
        .mockRejectedValueOnce({ code: "internal", message: "" })
        .mockResolvedValue([annotation(7)]),
    } satisfies AnnotationsApi;
    const source = createAnnotationSource(api);
    await expect(source.annotations(1, 0)).rejects.toBeDefined();
    expect(source.loaded(1, 0)).toBeUndefined();
    expect(await source.annotations(1, 0)).toEqual([annotation(7)]);
    expect(api.getPageAnnotations).toHaveBeenCalledTimes(2);
  });
});
