// Pure helpers for signature image cropping and typed signature sizing (B6-02).

export type CropBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

/**
 * Finds the bounding box of non-transparent pixels in an RGBA buffer,
 * expanded by `padding` pixels and clamped to the image dimensions.
 * Returns `null` if no pixels have alpha > 0.
 */
export function findCropBounds(
  pixels: Uint8ClampedArray | Uint8Array,
  width: number,
  height: number,
  padding = 12,
): CropBounds | null {
  let minX = width;
  let maxX = -1;
  let minY = height;
  let maxY = -1;

  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const alpha = pixels[(y * width + x) * 4 + 3];
      if (alpha !== undefined && alpha > 0) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }

  if (maxX === -1) {
    return null;
  }

  const x0 = Math.max(0, minX - padding);
  const y0 = Math.max(0, minY - padding);
  const x1 = Math.min(width, maxX + 1 + padding);
  const y1 = Math.min(height, maxY + 1 + padding);

  return {
    x: x0,
    y: y0,
    width: x1 - x0,
    height: y1 - y0,
  };
}

/**
 * Calculates font size for a typed signature name, scaling down if measured
 * width exceeds `maxWidth` (default 1024).
 */
export function calculateTypedFontSize(
  measuredWidth: number,
  baseFontSize = 72,
  maxWidth = 1024,
): number {
  if (measuredWidth <= maxWidth || measuredWidth <= 0) {
    return baseFontSize;
  }
  const scaled = Math.floor((baseFontSize * maxWidth) / measuredWidth);
  return Math.max(12, scaled);
}

/** Converts an HTML canvas element to PNG bytes. */
export async function defaultCanvasToPng(canvas: HTMLCanvasElement): Promise<number[]> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(async (blob) => {
      if (!blob) {
        reject(new Error("Canvas toBlob failed"));
        return;
      }
      try {
        const buffer = await blob.arrayBuffer();
        resolve(Array.from(new Uint8Array(buffer)));
      } catch (err) {
        reject(err);
      }
    }, "image/png");
  });
}
