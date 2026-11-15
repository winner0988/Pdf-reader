import { strings } from "@/i18n/zh-TW";
import type { PageSize, PdfDate } from "@/ipc/generated/contract";

export function formatPdfDate(date: PdfDate | null | undefined): string {
  if (!date) return "-";
  const pad = (n: number) => n.toString().padStart(2, "0");
  const ymd = `${date.year}-${pad(date.month)}-${pad(date.day)}`;
  const hms = `${pad(date.hour)}:${pad(date.minute)}:${pad(date.second)}`;
  if (date.tzOffsetMinutes === null || date.tzOffsetMinutes === undefined) {
    return `${ymd} ${hms}`;
  }
  if (date.tzOffsetMinutes === 0) {
    return `${ymd} ${hms} UTC`;
  }
  const sign = date.tzOffsetMinutes >= 0 ? "+" : "-";
  const absMinutes = Math.abs(date.tzOffsetMinutes);
  const tzH = Math.floor(absMinutes / 60);
  const tzM = absMinutes % 60;
  return `${ymd} ${hms} UTC${sign}${pad(tzH)}:${pad(tzM)}`;
}

export function formatSinglePageSize(size: PageSize): string {
  const wMm = Math.round((size.widthPt * 25.4) / 72);
  const hMm = Math.round((size.heightPt * 25.4) / 72);
  const wPt = Math.round(size.widthPt);
  const hPt = Math.round(size.heightPt);
  return `${wMm} × ${hMm} mm（${wPt} × ${hPt} pt）`;
}

export function formatPageSizes(pageSizes: PageSize[], t: typeof strings.properties): string {
  if (pageSizes.length === 0) return "-";
  const first = pageSizes[0];
  if (!first) return "-";
  const allSame = pageSizes.every(
    (s) => Math.abs(s.widthPt - first.widthPt) < 0.5 && Math.abs(s.heightPt - first.heightPt) < 0.5,
  );
  if (allSame) {
    return formatSinglePageSize(first);
  }
  return t.multipleSizes(formatSinglePageSize(first));
}
