import { pageElementId } from "@/features/shell/format";
import type { PageSize, Rotation, Zoom } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

type PageCanvasProps = {
  pages: PageSize[];
  zoom: Zoom;
  rotation: Rotation;
};

/**
 * Placeholder pages at their real proportions. MVP-07 replaces the boxes with rendered
 * rasters and virtual scrolling; fit modes are shown at 100% until then.
 */
export function PageCanvas({ pages, zoom, rotation }: PageCanvasProps) {
  const scale = (typeof zoom === "number" ? zoom : 100) / 100;
  const quarterTurn = rotation === 90 || rotation === 270;

  return (
    <div className="flex flex-col items-center gap-3 px-4 py-4">
      {pages.map((page, index) => {
        const width = (quarterTurn ? page.heightPt : page.widthPt) * scale;
        const height = (quarterTurn ? page.widthPt : page.heightPt) * scale;
        return (
          <div
            key={index}
            id={pageElementId(index + 1)}
            aria-label={strings.canvas.page(index + 1)}
            role="img"
            className="flex shrink-0 items-center justify-center bg-white text-sm text-neutral-400 shadow-sm ring-1 ring-black/10"
            style={{ width, height }}
          >
            {index + 1}
          </div>
        );
      })}
    </div>
  );
}
