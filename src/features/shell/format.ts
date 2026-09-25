import type { Zoom } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

export function formatZoom(zoom: Zoom): string {
  if (zoom === "fitWidth") return strings.toolbar.fitWidth;
  if (zoom === "fitPage") return strings.toolbar.fitPage;
  return strings.toolbar.zoomPercent(zoom);
}
