// Helpers and calculations for page marks and Bates numbering (B3-16).

import type {
  BatchOperation,
  HeaderFooterMarks,
  PageMarkPages,
  PageMarkPosition,
  PageMarks,
  PageNumberFormat,
  Rgb,
} from "@/ipc/generated/contract";
import { strings } from "@/i18n/zh-TW";

export const POSITIONS: PageMarkPosition[] = [
  "topLeft",
  "topCenter",
  "topRight",
  "bottomLeft",
  "bottomCenter",
  "bottomRight",
];

export function posLabel(pos: PageMarkPosition): string {
  const t = strings.pageMarks;
  switch (pos) {
    case "topLeft":
      return t.posTopLeft;
    case "topCenter":
      return t.posTopCenter;
    case "topRight":
      return t.posTopRight;
    case "bottomLeft":
      return t.posBottomLeft;
    case "bottomCenter":
      return t.posBottomCenter;
    case "bottomRight":
      return t.posBottomRight;
  }
}

export const PRESET_COLORS: { label: string; color: Rgb }[] = [
  { label: strings.pageMarks.colorBlack, color: { r: 0, g: 0, b: 0 } },
  { label: strings.pageMarks.colorGray, color: { r: 100, g: 100, b: 100 } },
  { label: strings.pageMarks.colorRed, color: { r: 200, g: 0, b: 0 } },
  { label: strings.pageMarks.colorBlue, color: { r: 0, g: 70, b: 180 } },
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
  pageNumberPosition: "bottomCenter",

  enableBates: false,
  batesPrefix: "",
  batesDigits: 6,
  batesStart: 1,
  batesPosition: "bottomRight",

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
  if (format.kind === "number") {
    return `${n}`;
  }
  if (format.kind === "page") {
    return `第 ${n} 頁`;
  }
  if (format.kind === "pageOfTotal") {
    return `第 ${n} 頁／共 ${total} 頁`;
  }
  if (format.kind === "custom") {
    return `${format.prefix}${n}${format.suffix}`;
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

/** Header or footer words as the worker takes them: one line, single spaces, none at the ends. */
export function oneLine(text: string): string {
  return text.replace(/\s+/g, " ").trim();
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

  const parts = trimmed
    .replace(/\s*([-~～])\s*/g, "$1")
    .split(/[,，\s]+/)
    .filter(Boolean);
  if (parts.length === 0) return null;

  const indices = new Set<number>();

  for (const part of parts) {
    if (/^\d+$/.test(part)) {
      const page = parseInt(part, 10);
      if (page < 1 || page > totalPages) return null;
      indices.add(page - 1);
    } else {
      const [, first, last] = /^(\d+)[-~～](\d+)$/.exec(part) ?? [];
      if (first === undefined || last === undefined) return null;
      const start = parseInt(first, 10);
      const end = parseInt(last, 10);
      if (start < 1 || end < start || end > totalPages) return null;
      for (let p = start; p <= end; p++) {
        indices.add(p - 1);
      }
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
    topLeft: 0,
    topCenter: 0,
    topRight: 0,
    bottomLeft: 0,
    bottomCenter: 0,
    bottomRight: 0,
  };

  if (form.enablePageNumber) {
    counts[form.pageNumberPosition]++;
  }

  if (form.enableBates) {
    counts[form.batesPosition]++;
  }

  if (form.enableHeaderFooter) {
    if (form.headerLeft.trim()) counts.topLeft++;
    if (form.headerCenter.trim()) counts.topCenter++;
    if (form.headerRight.trim()) counts.topRight++;
    if (form.footerLeft.trim()) counts.bottomLeft++;
    if (form.footerCenter.trim()) counts.bottomCenter++;
    if (form.footerRight.trim()) counts.bottomRight++;
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
    let fmt: PageNumberFormat = { kind: "number" };
    if (form.pageNumberFormatType === "Page") fmt = { kind: "page" };
    else if (form.pageNumberFormatType === "PageOfTotal") fmt = { kind: "pageOfTotal" };
    else if (form.pageNumberFormatType === "Custom") {
      fmt = {
        kind: "custom",
        prefix: form.customPrefix,
        suffix: form.customSuffix,
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
      topLeft: null,
      topCenter: null,
      topRight: null,
      bottomLeft: null,
      bottomCenter: null,
      bottomRight: null,
    };

    if (form.enableHeaderFooter) {
      if (form.headerLeft.trim()) slots.topLeft = form.headerLeft.trim();
      if (form.headerCenter.trim()) slots.topCenter = form.headerCenter.trim();
      if (form.headerRight.trim()) slots.topRight = form.headerRight.trim();
      if (form.footerLeft.trim()) slots.bottomLeft = form.footerLeft.trim();
      if (form.footerCenter.trim()) slots.bottomCenter = form.footerCenter.trim();
      if (form.footerRight.trim()) slots.bottomRight = form.footerRight.trim();
    }

    if (form.enablePageNumber) {
      slots[form.pageNumberPosition] = formatPn(seq);
    }

    if (form.enableBates) {
      slots[form.batesPosition] = formatBt(seq);
    }

    return slots;
  };

  const first = markedIndices[0] ?? 0;
  const last = markedIndices[markedCount - 1] ?? first;
  return {
    markedCount,
    firstPage: {
      pageNumber: first + 1,
      slots: buildSlots(0),
    },
    lastPage: {
      pageNumber: last + 1,
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

  let pages: PageMarkPages = { kind: "all" };
  if (form.pagesType === "AllButFirst") {
    pages = { kind: "allButFirst" };
  } else if (form.pagesType === "Custom") {
    pages = { kind: "custom", pages: markedIndices };
  }

  const pageNumber = form.enablePageNumber
    ? {
        format:
          form.pageNumberFormatType === "Number"
            ? ({ kind: "number" } as const)
            : form.pageNumberFormatType === "Page"
            ? ({ kind: "page" } as const)
            : form.pageNumberFormatType === "PageOfTotal"
            ? ({ kind: "pageOfTotal" } as const)
            : {
                kind: "custom" as const,
                prefix: form.customPrefix,
                suffix: form.customSuffix,
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
        headerLeft: oneLine(form.headerLeft) || null,
        headerCenter: oneLine(form.headerCenter) || null,
        headerRight: oneLine(form.headerRight) || null,
        footerLeft: oneLine(form.footerLeft) || null,
        footerCenter: oneLine(form.footerCenter) || null,
        footerRight: oneLine(form.footerRight) || null,
      }
    : null;

  if (!pageNumber && !bates && !headerFooter) return null;

  return {
    fontSize: Math.min(72, Math.max(6, form.fontSize)),
    color: form.color,
    marginPt: Math.min(144, Math.max(0, form.marginPt)),
    pages,
    pageNumber,
    bates,
    headerFooter,
  };
}

/** Checks if there is any problem with the form for batch processing. */
export function pageMarksProblemWith(form: PageMarksForm): string | null {
  const conflicts = findPositionConflicts(form);
  if (conflicts.length > 0) {
    return strings.pageMarks.conflictError;
  }
  const noMarks =
    !form.enablePageNumber &&
    !form.enableBates &&
    (!form.enableHeaderFooter ||
      (!form.headerLeft.trim() &&
        !form.headerCenter.trim() &&
        !form.headerRight.trim() &&
        !form.footerLeft.trim() &&
        !form.footerCenter.trim() &&
        !form.footerRight.trim()));
  if (noMarks) {
    return strings.pageMarks.noMarksError;
  }
  if (form.pagesType === "Custom") {
    const parsed = parsePageRanges(form.customPagesString, 10000);
    if (!parsed || parsed.length === 0) {
      return strings.pageMarks.pagesCustomError;
    }
  }
  return null;
}

/** Builds the BatchOperation for page marks from form state. */
export function batchPageMarksOf(form: PageMarksForm): BatchOperation {
  let pages: PageMarkPages = { kind: "all" };
  if (form.pagesType === "AllButFirst") {
    pages = { kind: "allButFirst" };
  } else if (form.pagesType === "Custom") {
    const parsed = parsePageRanges(form.customPagesString, 10000) ?? [];
    pages = { kind: "custom", pages: parsed };
  }

  const pageNumber = form.enablePageNumber
    ? {
        format:
          form.pageNumberFormatType === "Number"
            ? ({ kind: "number" } as const)
            : form.pageNumberFormatType === "Page"
            ? ({ kind: "page" } as const)
            : form.pageNumberFormatType === "PageOfTotal"
            ? ({ kind: "pageOfTotal" } as const)
            : {
                kind: "custom" as const,
                prefix: form.customPrefix,
                suffix: form.customSuffix,
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
        headerLeft: oneLine(form.headerLeft) || null,
        headerCenter: oneLine(form.headerCenter) || null,
        headerRight: oneLine(form.headerRight) || null,
        footerLeft: oneLine(form.footerLeft) || null,
        footerCenter: oneLine(form.footerCenter) || null,
        footerRight: oneLine(form.footerRight) || null,
      }
    : null;

  return {
    kind: "pageMarks",
    marks: {
      fontSize: Math.min(72, Math.max(6, form.fontSize)),
      color: form.color,
      marginPt: Math.min(144, Math.max(0, form.marginPt)),
      pages,
      pageNumber,
      bates,
      headerFooter,
    },
  };
}
