import { describe, expect, it } from "vitest";

import { MAX_PRINT_PAGES, pagesToPrint, parsePageList } from "@/features/print/range";

describe("print ranges", () => {
  it("reads page lists the way people write them", () => {
    expect(parsePageList("1-3, 5", 10)).toEqual([0, 1, 2, 4]);
    expect(parsePageList(" 5，1 - 2、3 ", 10)).toEqual([0, 1, 2, 4]);
    expect(parsePageList("7~9 9 8", 10)).toEqual([6, 7, 8]);
    expect(parsePageList("10", 10)).toEqual([9]);
  });

  it("rejects anything that is not a page of the document", () => {
    for (const text of ["", " ", "0", "11", "3-2", "1-11", "a", "1-", "-3", "1,,x", "1.5"]) {
      expect(parsePageList(text, 10), text).toBeNull();
    }
  });

  it("prints all pages, the current one or a list", () => {
    expect(pagesToPrint({ kind: "all" }, 3, 2)).toEqual({ pages: [0, 1, 2] });
    expect(pagesToPrint({ kind: "current" }, 3, 2)).toEqual({ pages: [1] });
    expect(pagesToPrint({ kind: "pages", text: "3, 1" }, 3, 2)).toEqual({ pages: [0, 2] });
    expect(pagesToPrint({ kind: "pages", text: "4" }, 3, 2)).toEqual({ error: "invalid" });
  });

  it("prints at most so many pages at once", () => {
    const many = MAX_PRINT_PAGES + 50;
    expect(pagesToPrint({ kind: "all" }, many, 1)).toEqual({ error: "tooMany" });
    expect(pagesToPrint({ kind: "pages", text: `1-${many}` }, many, 1)).toEqual({ error: "tooMany" });
    expect(pagesToPrint({ kind: "pages", text: `1-${MAX_PRINT_PAGES}` }, many, 1)).toMatchObject({
      pages: expect.arrayContaining([0, MAX_PRINT_PAGES - 1]),
    });
  });
});
