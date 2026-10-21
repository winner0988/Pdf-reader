import { describe, expect, it } from "vitest";

import {
  buildPageMarksPayload,
  calculatePreview,
  DEFAULT_PAGE_MARKS_FORM,
  findPositionConflicts,
  formatBates,
  formatPageNumber,
  isBatesOverflow,
  parsePageRanges,
  type PageMarksForm,
} from "./pageMarks";

describe("pageMarks formatting and calculations", () => {
  it("formats page numbers in all 4 formats", () => {
    expect(formatPageNumber(1, 10, "Number")).toBe("1");
    expect(formatPageNumber(5, 10, "Page")).toBe("第 5 頁");
    expect(formatPageNumber(2, 9, "PageOfTotal")).toBe("第 2 頁／共 9 頁");
    expect(
      formatPageNumber(3, 10, { Custom: { prefix: "[Doc ", suffix: "]" } }),
    ).toBe("[Doc 3]");
  });

  it("formats Bates numbers with zero padding and prefix", () => {
    expect(formatBates(1, "ABC", 6)).toBe("ABC000001");
    expect(formatBates(999, "EXHIBIT-", 5)).toBe("EXHIBIT-00999");
    expect(formatBates(123456, "", 6)).toBe("123456");
  });

  it("detects Bates number overflow", () => {
    // 1 digit can represent 0-9. 10 pages starting at 1 -> last is 10 >= 10 -> overflow!
    expect(isBatesOverflow(1, 1, 10)).toBe(true);
    // 9 pages starting at 1 -> last is 9 < 10 -> ok
    expect(isBatesOverflow(1, 1, 9)).toBe(false);
    // 6 digits: 1000 + 10 - 1 = 1009 < 1,000,000 -> ok
    expect(isBatesOverflow(1000, 6, 10)).toBe(false);
    // 2 digits: 95 + 10 - 1 = 104 >= 100 -> overflow!
    expect(isBatesOverflow(95, 2, 10)).toBe(true);
  });

  it("parses page ranges correctly and rejects out of bounds", () => {
    expect(parsePageRanges("1, 3-5, 8", 10)).toEqual([0, 2, 3, 4, 7]);
    expect(parsePageRanges("10", 10)).toEqual([9]);
    expect(parsePageRanges("1-3, 2-4", 10)).toEqual([0, 1, 2, 3]);

    // Invalid formats or out of bounds
    expect(parsePageRanges("", 10)).toBeNull();
    expect(parsePageRanges("abc", 10)).toBeNull();
    expect(parsePageRanges("0, 2", 10)).toBeNull();
    expect(parsePageRanges("1-15", 10)).toBeNull();
    expect(parsePageRanges("5-2", 10)).toBeNull();
  });

  it("detects slot collisions between marks", () => {
    const form: PageMarksForm = {
      ...DEFAULT_PAGE_MARKS_FORM,
      enablePageNumber: true,
      pageNumberPosition: "BottomCenter",
      enableBates: true,
      batesPosition: "BottomCenter", // Collision!
    };
    expect(findPositionConflicts(form)).toEqual(["BottomCenter"]);

    // Distinct positions -> no collision
    const noCollision: PageMarksForm = {
      ...form,
      batesPosition: "BottomRight",
    };
    expect(findPositionConflicts(noCollision)).toEqual([]);

    // Collision with header/footer
    const hfCollision: PageMarksForm = {
      ...noCollision,
      enableHeaderFooter: true,
      footerRight: "CONFIDENTIAL", // Collision with Bates at BottomRight!
    };
    expect(findPositionConflicts(hfCollision)).toEqual(["BottomRight"]);
  });

  it("calculates preview text for first and last marked pages", () => {
    // 10 pages total, AllButFirst (9 marked pages)
    // First marked page is page 2 (index 1), page number starts at 1
    // Rule: total is start + marked_count - 1 = 1 + 9 - 1 = 9
    // So first page has "第 1 頁／共 9 頁", last page has "第 9 頁／共 9 頁"
    const form: PageMarksForm = {
      ...DEFAULT_PAGE_MARKS_FORM,
      pagesType: "AllButFirst",
      enablePageNumber: true,
      pageNumberFormatType: "PageOfTotal",
      pageNumberStart: 1,
      pageNumberPosition: "BottomCenter",
      enableBates: true,
      batesPrefix: "BATES-",
      batesDigits: 5,
      batesStart: 100,
      batesPosition: "BottomRight",
      enableHeaderFooter: true,
      headerLeft: "TOP SECRET",
    };

    const preview = calculatePreview(form, 10);
    expect(preview.markedCount).toBe(9);
    expect(preview.firstPage).not.toBeNull();
    expect(preview.lastPage).not.toBeNull();

    // First marked page is page 2 (1-based)
    expect(preview.firstPage?.pageNumber).toBe(2);
    expect(preview.firstPage?.slots.BottomCenter).toBe("第 1 頁／共 9 頁");
    expect(preview.firstPage?.slots.BottomRight).toBe("BATES-00100");
    expect(preview.firstPage?.slots.TopLeft).toBe("TOP SECRET");

    // Last marked page is page 10 (1-based)
    expect(preview.lastPage?.pageNumber).toBe(10);
    expect(preview.lastPage?.slots.BottomCenter).toBe("第 9 頁／共 9 頁");
    expect(preview.lastPage?.slots.BottomRight).toBe("BATES-00108");
    expect(preview.lastPage?.slots.TopLeft).toBe("TOP SECRET");
  });

  it("builds payload only when valid and without conflicts", () => {
    const validForm: PageMarksForm = {
      ...DEFAULT_PAGE_MARKS_FORM,
      enablePageNumber: true,
      pageNumberPosition: "BottomCenter",
    };
    const payload = buildPageMarksPayload(validForm, 5);
    expect(payload).not.toBeNull();
    expect(payload?.pages).toBe("All");
    expect(payload?.page_number?.position).toBe("BottomCenter");

    // Conflict returns null
    const conflictForm: PageMarksForm = {
      ...validForm,
      enableBates: true,
      batesPosition: "BottomCenter",
    };
    expect(buildPageMarksPayload(conflictForm, 5)).toBeNull();

    // Bates overflow returns null
    const overflowForm: PageMarksForm = {
      ...validForm,
      enablePageNumber: false,
      enableBates: true,
      batesDigits: 1,
      batesStart: 1,
      batesPosition: "BottomRight",
    };
    expect(buildPageMarksPayload(overflowForm, 10)).toBeNull();
  });
});
