import { describe, expect, it, vi } from "vitest";

import { createTextSource } from "@/features/text/source";
import type { PageText } from "@/ipc/generated/contract";

const textOf = (pageIndex: number): PageText => ({
  lines: [
    {
      text: `page ${pageIndex}`,
      quad: { ul: { x: 0, y: 0 }, ur: { x: 6, y: 0 }, ll: { x: 0, y: 1 }, lr: { x: 6, y: 1 } },
      edges: [0, 1, 2, 3, 4, 5, 6],
    },
  ],
  truncated: false,
});

function api() {
  return { getPageText: vi.fn((_doc: number, pageIndex: number) => Promise.resolve(textOf(pageIndex))) };
}

describe("text source", () => {
  it("asks for a page once and has its text at hand once it arrived", async () => {
    const main = api();
    const source = createTextSource(main);
    expect(source.loaded(1, 0)).toBeUndefined();
    const first = source.text(1, 0);
    expect(source.text(1, 0)).toBe(first);
    await first;
    expect(source.loaded(1, 0)).toEqual(textOf(0));
    expect(main.getPageText).toHaveBeenCalledTimes(1);
  });

  it("starts over for another document", async () => {
    const main = api();
    const source = createTextSource(main);
    await source.text(1, 0);
    await source.text(2, 0);
    expect(source.loaded(1, 0)).toBeUndefined();
    expect(main.getPageText).toHaveBeenCalledTimes(2);
  });

  it("keeps the most recently used pages", async () => {
    const main = api();
    const source = createTextSource(main, 2);
    await source.text(1, 0);
    await source.text(1, 1);
    await source.text(1, 0);
    await source.text(1, 2);
    // Page 1 was the least recently used.
    expect(source.loaded(1, 1)).toBeUndefined();
    expect(source.loaded(1, 0)).toBeDefined();
    expect(source.loaded(1, 2)).toBeDefined();
  });

  it("forgets a failure so that the page is asked again", async () => {
    const main = api();
    main.getPageText.mockRejectedValueOnce({ code: "workerCrashed", message: "" });
    const source = createTextSource(main);
    await expect(source.text(1, 0)).rejects.toMatchObject({ code: "workerCrashed" });
    await expect(source.text(1, 0)).resolves.toEqual(textOf(0));
  });

  it("reads pages for copying without keeping them", async () => {
    const main = api();
    const source = createTextSource(main, 1);
    await source.text(1, 0);
    await expect(source.textOnce(1, 5)).resolves.toEqual(textOf(5));
    expect(source.loaded(1, 5)).toBeUndefined();
    // The page being read is still there, and served from what is kept.
    expect(source.loaded(1, 0)).toBeDefined();
    await source.textOnce(1, 0);
    expect(main.getPageText).toHaveBeenCalledTimes(2);
  });
});
