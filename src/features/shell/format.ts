import type { Zoom } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

/** DOM id of a page placeholder, used to scroll to a page. */
export const pageElementId = (page: number) => `page-${page}`;

export function formatZoom(zoom: Zoom): string {
  if (zoom === "fitWidth") return strings.toolbar.fitWidth;
  if (zoom === "fitPage") return strings.toolbar.fitPage;
  return strings.toolbar.zoomPercent(zoom);
}
