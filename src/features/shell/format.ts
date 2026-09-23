import { FINDING_KINDS, type SecurityFinding, type Zoom } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

/** DOM id of a page placeholder, used to scroll to a page. */
export const pageElementId = (page: number) => `page-${page}`;

export function formatZoom(zoom: Zoom): string {
  if (zoom === "fitWidth") return strings.toolbar.fitWidth;
  if (zoom === "fitPage") return strings.toolbar.fitPage;
  return strings.toolbar.zoomPercent(zoom);
}

/** Banner sentence: total count and up to three kinds in display order. */
export function bannerSummary(findings: SecurityFinding[]): string {
  const ordered = [...findings].sort(
    (a, b) => FINDING_KINDS.indexOf(a.kind) - FINDING_KINDS.indexOf(b.kind),
  );
  const total = ordered.reduce((sum, finding) => sum + finding.count, 0);
  const names = ordered.slice(0, 3).map((finding) => strings.findings[finding.kind].name);
  return strings.banner.summary(total, names, ordered.length > 3);
}
