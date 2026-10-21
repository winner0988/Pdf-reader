// Helpers and calculations for page marks and Bates numbering (B3-16).

import type {
  HeaderFooterMarks,
  PageMarkPages,
  PageMarkPosition,
  PageMarks,
  PageNumberFormat,
  Rgb,
} from "@/ipc/generated/contract";

export const POSITIONS: PageMarkPosition[] = [
  "TopLeft",
  "TopCenter",
  "TopRight",
  "BottomLeft",
  "BottomCenter",
  "BottomRight",
];

export const PRESET_COLORS: { label: string; color: Rgb }[] = [
  { label: "黑色", color: { r: 0, g: 0, b: 0 } },
  { label: "灰色", color: { r: 100, g: 100, b: 100 } },
  { label: "紅色", color: { r: 200, g: 0, b: 0 } },
  { label: "藍色", color: { r: 0, g: 70, b: 180 } },
];

export type PageMarksForm = {
  // Common settings
  fontSize: number;
  color: Rgb;
  marginPt: number;
  pagesType: "All" | "AllButFirst" | "Custom";
  customPagesString: string;

  // Page numbers section
  enablePageNumber: boolean;
  pageNumberFormatType: "Number" | "Page" | "PageOfTotal" | "Custom";
  customPrefix: string;
  customSuffix: string;
  pageNumberStart: number;
  pageNumberPosition: PageMarkPosition;

  // Bates section
  enableBates: boolean;
  batesPrefix: string;
  batesDigits: number;
  batesStart: number;
  batesPosition: PageMarkPosition;

  // Header and footer section
  enableHeaderFooter: boolean;
  headerLeft: string;
  headerCenter: string;
  headerRight: string;
  footerLeft: string;
  footerCenter: string;
  footerRight: string;
};

export const DEFAULT_PAGE_MARKS_FORM: PageMarksForm = {
  fontSize: 10,
  color: { r: 0, g: 0, b: 0 },
  marginPt: 36,
  pagesType: "All",
  customPagesString: "",

  enablePageNumber: true,
  pageNumberFormatType: "PageOfTotal",
  customPrefix: "",
  customSuffix: "",
  pageNumberStart: 1,
  pageNumberPosition: "BottomCenter",

  enableBates: false,
  batesPrefix: "",
  batesDigits: 6,
  batesStart: 1,
  batesPosition: "BottomRight",

  enableHeaderFooter: false,
  headerLeft: "",
  headerCenter: "",
  headerRight: "",
  footerLeft: "",
  footerCenter: "",
  footerRight: "",
};

/**
 * Formats page number text according to `format` and the computed sequence numbers.
 */
export function formatPageNumber(
  n: number,
  total: number,
  format: PageNumberFormat,
): string {
  if (format === "Number") {
    return `${n}`;
  }
  if (format === "Page") {
    return `第 ${n} 頁`;
  }
  if (format === "PageOfTotal") {
    return `第 ${n} 頁／共 ${total} 頁`;
  }
  if (typeof format === "object" && "Custom" in format) {
    return `${format.Custom.prefix}${n}${format.Custom.suffix}`;
  }
  return `${n}`;
}

/**
 * Formats a Bates number with prefix and zero-padded digits.
 */
export function formatBates(
  n: number,
  prefix: string,
  digits: number,
): string {
  const padded = String(Math.max(0, Math.floor(n))).padStart(digits, "0");
  return `${prefix}${padded}`;
}

/**
 * Parses user input page range string (e.g. "1-5, 8, 11-13") into sorted 0-based page indices.
 * Returns null if the string is invalid or any page number exceeds `totalPages`.
 */
export function parsePageRanges(
  input: string,
  totalPages: number,
): number[] | null {
  const trimmed = input.trim();
  if (!trimmed) return null;

  const parts = trimmed.split(/[,，\s]+/).filter(Boolean);
  if (parts.length === 0) return null;

  const indices = new Set<number>();

  for (const part of parts) {
    if (/^\d+$/.test(part)) {
      const page = parseInt(part, 10);
      if (page < 1 || page > totalPages) return null;
      indices.add(page - 1);
    } else if (/^(\d+)\s*[-~～]\s*(\d+)$/.test(part)) {
      const match = part.match(/^(\d+)\s*[-~～]\s*(\d+)$/);
      if (!match) return null;
      const start = parseInt(match[1], 10);
      const end = parseInt(match[2], 10);
      if (start < 1 || end < start || end > totalPages) return null;
      for (let p = start; p <= end; p++) {
        indices.add(p - 1);
      }
    } else {
      return null;
    }
  }

  if (indices.size === 0 || indices.size > 10000) return null;

  return Array.from(indices).sort((a, b) => a - b);
}

/**
 * Returns the list of 0-based page indices that will receive marks.
 */
export function getMarkedPageIndices(
  pagesType: "All" | "AllButFirst" | "Custom",
  customPagesString: string,
  totalPages: number,
): number[] {
  if (totalPages <= 0) return [];
  if (pagesType === "All") {
    return Array.from({ length: totalPages }, (_, i) => i);
  }
  if (pagesType === "AllButFirst") {
    return Array.from({ length: Math.max(0, totalPages - 1) }, (_, i) => i + 1);
  }
  return parsePageRanges(customPagesString, totalPages) ?? [];
}

/**
 * Finds all slots where more than one mark has been placed.
 */
export function findPositionConflicts(form: PageMarksForm): PageMarkPosition[] {
  const counts: Record<PageMarkPosition, number> = {
    TopLeft: 0,
    TopCenter: 0,
    TopRight: 0,
    BottomLeft: 0,
    BottomCenter: 0,
    BottomRight: 0,
  };

  if (form.enablePageNumber) {
    counts[form.pageNumberPosition]++;
  }

  if (form.enableBates) {
    counts[form.batesPosition]++;
  }

  if (form.enableHeaderFooter) {
    if (form.headerLeft.trim()) counts.TopLeft++;
    if (form.headerCenter.trim()) counts.TopCenter++;
    if (form.headerRight.trim()) counts.TopRight++;
    if (form.footerLeft.trim()) counts.BottomLeft++;
    if (form.footerCenter.trim()) counts.BottomCenter++;
    if (form.footerRight.trim()) counts.BottomRight++;
  }

  return POSITIONS.filter((pos) => counts[pos] > 1);
}

export type PagePreviewSlots = Record<PageMarkPosition, string | null>;

export type MarksPreview = {
  markedCount: number;
  firstPage: {
    pageNumber: number; // 1-based display
    slots: PagePreviewSlots;
  } | null;
  lastPage: {
    pageNumber: number; // 1-based display
    slots: PagePreviewSlots;
  } | null;
};

/**
 * Calculates the preview texts for the first and last marked pages.
 */
export function calculatePreview(
  form: PageMarksForm,
  totalPages: number,
): MarksPreview {
  const markedIndices = getMarkedPageIndices(
    form.pagesType,
    form.customPagesString,
    totalPages,
  );
  const markedCount = markedIndices.length;

  if (markedCount === 0) {
    return { markedCount: 0, firstPage: null, lastPage: null };
  }

  const formatPn = (seq: number): string => {
    const n = Math.max(1, form.pageNumberStart) + seq;
    const total = Math.max(1, form.pageNumberStart) + markedCount - 1;
    let fmt: PageNumberFormat = "Number";
    if (form.pageNumberFormatType === "Page") fmt = "Page";
    else if (form.pageNumberFormatType === "PageOfTotal") fmt = "PageOfTotal";
    else if (form.pageNumberFormatType === "Custom") {
      fmt = {
        Custom: {
          prefix: form.customPrefix,
          suffix: form.customSuffix,
        },
      };
    }
    return formatPageNumber(n, total, fmt);
  };

  const formatBt = (seq: number): string => {
    const n = Math.max(0, form.batesStart) + seq;
    return formatBates(n, form.batesPrefix, form.batesDigits);
  };

  const buildSlots = (seq: number): PagePreviewSlots => {
    const slots: PagePreviewSlots = {
      TopLeft: null,
      TopCenter: null,
      TopRight: null,
      BottomLeft: null,
      BottomCenter: null,
      BottomRight: null,
    };

    if (form.enableHeaderFooter) {
      if (form.headerLeft.trim()) slots.TopLeft = form.headerLeft.trim();
      if (form.headerCenter.trim()) slots.TopCenter = form.headerCenter.trim();
      if (form.headerRight.trim()) slots.TopRight = form.headerRight.trim();
      if (form.footerLeft.trim()) slots.BottomLeft = form.footerLeft.trim();
      if (form.footerCenter.trim()) slots.BottomCenter = form.footerCenter.trim();
      if (form.footerRight.trim()) slots.BottomRight = form.footerRight.trim();
    }

    if (form.enablePageNumber) {
      slots[form.pageNumberPosition] = formatPn(seq);
    }

    if (form.enableBates) {
      slots[form.batesPosition] = formatBt(seq);
    }

    return slots;
  };

  return {
    markedCount,
    firstPage: {
      pageNumber: markedIndices[0] + 1,
      slots: buildSlots(0),
    },
    lastPage: {
      pageNumber: markedIndices[markedCount - 1] + 1,
      slots: buildSlots(markedCount - 1),
    },
  };
}

/**
 * Checks whether Bates number would overflow the available digits.
 */
export function isBatesOverflow(
  batesStart: number,
  batesDigits: number,
  markedCount: number,
): boolean {
  if (markedCount <= 0) return false;
  const lastNumber = batesStart + markedCount - 1;
  const maxAllowed = Math.pow(10, batesDigits);
  return lastNumber >= maxAllowed;
}

/**
 * Builds the contract-compatible `PageMarks` object from form state.
 */
export function buildPageMarksPayload(
  form: PageMarksForm,
  totalPages: number,
): PageMarks | null {
  const conflicts = findPositionConflicts(form);
  if (conflicts.length > 0) return null;

  const markedIndices = getMarkedPageIndices(
    form.pagesType,
    form.customPagesString,
    totalPages,
  );
  if (markedIndices.length === 0) return null;

  if (
    form.enableBates &&
    isBatesOverflow(form.batesStart, form.batesDigits, markedIndices.length)
  ) {
    return null;
  }

  let pages: PageMarkPages = "All";
  if (form.pagesType === "AllButFirst") {
    pages = "AllButFirst";
  } else if (form.pagesType === "Custom") {
    pages = { Custom: { pages: markedIndices } };
  }

  const pageNumber = form.enablePageNumber
    ? {
        format:
          form.pageNumberFormatType === "Number"
            ? ("Number" as const)
            : form.pageNumberFormatType === "Page"
            ? ("Page" as const)
            : form.pageNumberFormatType === "PageOfTotal"
            ? ("PageOfTotal" as const)
            : {
                Custom: {
                  prefix: form.customPrefix,
                  suffix: form.customSuffix,
                },
              },
        start: Math.max(1, form.pageNumberStart),
        position: form.pageNumberPosition,
      }
    : null;

  const bates = form.enableBates
    ? {
        prefix: form.batesPrefix,
        digits: Math.min(12, Math.max(1, form.batesDigits)),
        start: Math.max(0, form.batesStart),
        position: form.batesPosition,
      }
    : null;

  const headerFooter: HeaderFooterMarks | null = form.enableHeaderFooter
    ? {
        header_left: form.headerLeft.trim() || null,
        header_center: form.headerCenter.trim() || null,
        header_right: form.headerRight.trim() || null,
        footer_left: form.footerLeft.trim() || null,
        footer_center: form.footerCenter.trim() || null,
        footer_right: form.footerRight.trim() || null,
      }
    : null;

  if (!pageNumber && !bates && !headerFooter) return null;

  return {
    fontSize: Math.min(72, Math.max(6, form.fontSize)),
    color: form.color,
    marginPt: Math.min(144, Math.max(0, form.marginPt)),
    pages,
    page_number: pageNumber,
    bates,
    header_footer: headerFooter,
  };
}
