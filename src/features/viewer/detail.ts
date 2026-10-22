// Pages zoomed far in (B3-09, docs/architecture/rendering.md). A page whose raster is much
// larger than the screen is drawn in two layers: the whole page at a lower resolution, and over
// it only the part on screen at full resolution. Transferring a raster from the main process
// costs about as much as rendering it, so a page several screens large would take a second to
// appear sharp, and beyond the raster limits it could not be sharp at all.

import type { PageSize } from "@/features/shell/model";
import { renderScale } from "@/features/viewer/layout";
import { LIMITS, type RasterRegion, type Rotation as ContractRotation } from "@/ipc/generated/contract";

export type RasterSize = { width: number; height: number };

/** A rectangle in a page box's CSS pixels, from its top left corner. */
export type BoxRect = { left: number; top: number; width: number; height: number };

/** Pages larger than this many screens are drawn in two layers. */
const DETAIL_FROM_SCREENS = 2;

/** The part drawn sharp reaches this fraction of the part on screen beyond it on every side. */
const DETAIL_MARGIN = 1 / 16;

/**
 * Pixel size of a page's whole raster at `scale`, as both processes work it out
 * (`ipc_contract::raster`): each side in points times the scale, both as 32-bit floats
 * multiplied exactly, rounded up, at least one pixel.
 */
export function rasterSize(page: PageSize, rotation: ContractRotation, scale: number): RasterSize {
  const toPx = (points: number) => Math.max(Math.ceil(Math.fround(points) * Math.fround(scale)), 1);
  const width = toPx(page.widthPt);
  const height = toPx(page.heightPt);
  return rotation === "cw90" || rotation === "cw270" ? { width: height, height: width } : { width, height };
}

/**
 * The scale of the whole page underneath when a page is drawn in two layers at zoom `factor`,
 * or null when it is drawn whole. Underneath is the page at half, a quarter, ... of the zoom,
 * the first that is no larger than the screen: the scales the view had before zooming in, so
 * the page is usually drawn already, or still in the main process's cache.
 */
export function detailBaseScale(
  page: PageSize,
  rotation: ContractRotation,
  factor: number,
  devicePixelRatio: number,
  screen: RasterSize,
): number | null {
  const screenPixels = screen.width * screen.height;
  if (!(screenPixels > 0)) return null;
  const whole = rasterSize(page, rotation, renderScale(factor, devicePixelRatio));
  const pixels = whole.width * whole.height;
  const fits =
    Math.max(whole.width, whole.height) <= LIMITS.maxRasterSidePx && pixels <= LIMITS.maxRasterPixels;
  if (fits && pixels <= DETAIL_FROM_SCREENS * screenPixels) return null;
  // Too large for a region's coordinates: drawn whole, at the lower scale that fits.
  if (Math.max(whole.width, whole.height) > LIMITS.maxRegionPageSidePx) return null;
  let halves = 1;
  let base = renderScale(factor / 2, devicePixelRatio);
  for (;;) {
    const size = rasterSize(page, rotation, base);
    if (size.width * size.height <= screenPixels || base <= LIMITS.minRenderScale || halves >= 32) {
      return base;
    }
    halves++;
    base = renderScale(factor / 2 ** halves, devicePixelRatio);
  }
}

/**
 * The part of a page's raster to draw sharp: `view` (the part of the page's box on screen)
 * with `margin` (a fraction of its size) around it, within the page and within the raster
 * limits around its middle. `box` is the page's displayed size and `whole` its raster's.
 */
export function detailRegion(
  view: BoxRect,
  box: RasterSize,
  whole: RasterSize,
  margin = DETAIL_MARGIN,
): RasterRegion | null {
  if (!(view.width > 0 && view.height > 0 && box.width > 0 && box.height > 0)) return null;
  const across = (start: number, length: number, boxLength: number, wholeLength: number, max: number) => {
    const ratio = wholeLength / boxLength;
    let from = Math.max(Math.floor((start - length * margin) * ratio), 0);
    let to = Math.min(Math.ceil((start + length * (1 + margin)) * ratio), wholeLength);
    if (to - from > max) {
      from = Math.max(Math.round((from + to - max) / 2), 0);
      to = Math.min(from + max, wholeLength);
    }
    return to > from ? { from, to } : null;
  };
  const side = LIMITS.maxRasterSidePx;
  const x = across(view.left, view.width, box.width, whole.width, side);
  const y = across(view.top, view.height, box.height, whole.height, side);
  if (!x || !y) return null;
  let region = { x: x.from, y: y.from, width: x.to - x.from, height: y.to - y.from };
  if (region.width * region.height > LIMITS.maxRasterPixels) {
    // Shrunk around its middle (a screen larger than 4096 x 4096 pixels).
    const shrink = Math.sqrt(LIMITS.maxRasterPixels / (region.width * region.height));
    const width = Math.floor(region.width * shrink);
    const height = Math.floor(region.height * shrink);
    region = {
      x: region.x + Math.floor((region.width - width) / 2),
      y: region.y + Math.floor((region.height - height) / 2),
      width,
      height,
    };
  }
  return region;
}

/** Whether `outer` covers all of `inner`. */
export function covers(outer: RasterRegion, inner: RasterRegion): boolean {
  return (
    outer.x <= inner.x &&
    outer.y <= inner.y &&
    outer.x + outer.width >= inner.x + inner.width &&
    outer.y + outer.height >= inner.y + inner.height
  );
}

/** The part of a page's box on screen, in the box's CSS pixels; null when none of it is. */
export function visiblePart(
  box: { top: number; width: number; height: number },
  left: number,
  viewport: { top: number; left: number; width: number; height: number },
): BoxRect | null {
  const x0 = Math.max(viewport.left, left);
  const x1 = Math.min(viewport.left + viewport.width, left + box.width);
  const y0 = Math.max(viewport.top, box.top);
  const y1 = Math.min(viewport.top + viewport.height, box.top + box.height);
  if (x1 <= x0 || y1 <= y0) return null;
  return { left: x0 - left, top: y0 - box.top, width: x1 - x0, height: y1 - y0 };
}
