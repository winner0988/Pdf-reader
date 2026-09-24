import { describe, expect, it, vi } from "vitest";

import { createLinkSource, type LinksApi } from "@/features/links/source";
import type { PageLink } from "@/ipc/generated/contract";

const link = (pageIndex: number): PageLink => ({
  id: { pageIndex, index: 0 },
  rect: { x0: 0, y0: 0, x1: 10, y1: 10 },
  target: { kind: "page", pageIndex: 0, x: null, y: null },
});

describe("createLinkSource", () => {
  it("asks for each page once per document", async () => {
    const api = { getPageLinks: vi.fn((_doc: number, page: number) => Promise.resolve([link(page)])) } satisfies LinksApi;
    const source = createLinkSource(api);

    expect(await source.links(1, 0)).toEqual([link(0)]);
    await source.links(1, 0);
    await source.links(1, 3);
    expect(api.getPageLinks.mock.calls).toEqual([
      [1, 0],
      [1, 3],
    ]);

    // Another document starts afresh.
    await source.links(2, 0);
    await source.links(1, 0);
    expect(api.getPageLinks).toHaveBeenCalledTimes(4);
  });

  it("asks again after a failure", async () => {
    const api = {
      getPageLinks: vi
        .fn<LinksApi["getPageLinks"]>()
        .mockRejectedValueOnce({ code: "workerCrashed", message: "" })
        .mockResolvedValue([link(0)]),
    } satisfies LinksApi;
    const source = createLinkSource(api);

    await expect(source.links(1, 0)).rejects.toMatchObject({ code: "workerCrashed" });
    expect(await source.links(1, 0)).toEqual([link(0)]);
    expect(api.getPageLinks).toHaveBeenCalledTimes(2);
  });
});
