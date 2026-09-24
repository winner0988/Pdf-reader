import { LIMITS, RASTER } from "@/ipc/generated/contract";

/** A rendered page: opaque RGBA8, rows top to bottom. */
export type RasterImage = {
  width: number;
  height: number;
  /** A view into the response buffer (no copy); ready for `new ImageData(...)`. */
  pixels: Uint8ClampedArray<ArrayBuffer>;
};

export class RasterFormatError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RasterFormatError";
  }
}

/**
 * Decodes the raw response of the `render_page` command.
 * Layout: see crates/ipc_contract/src/raster.rs and docs/architecture/ipc-contract.md.
 */
export function decodeRaster(buffer: ArrayBuffer): RasterImage {
  if (buffer.byteLength < RASTER.headerBytes) {
    throw new RasterFormatError("response is shorter than the raster header");
  }
  const view = new DataView(buffer);

  for (let i = 0; i < RASTER.magic.length; i++) {
    if (view.getUint8(i) !== RASTER.magic.charCodeAt(i)) {
      throw new RasterFormatError("bad raster magic");
    }
  }
  if (view.getUint16(4, true) !== RASTER.formatRgba8) {
    throw new RasterFormatError("unsupported raster format");
  }
  if (view.getUint16(6, true) !== 0) {
    throw new RasterFormatError("reserved raster header field is not zero");
  }

  const width = view.getUint32(8, true);
  const height = view.getUint32(12, true);
  if (
    width === 0 ||
    height === 0 ||
    width > LIMITS.maxRasterSidePx ||
    height > LIMITS.maxRasterSidePx ||
    width * height > LIMITS.maxRasterPixels
  ) {
    throw new RasterFormatError("raster size is out of range");
  }
  if (buffer.byteLength !== RASTER.headerBytes + width * height * 4) {
    throw new RasterFormatError("raster length does not match its size");
  }

  return { width, height, pixels: new Uint8ClampedArray(buffer, RASTER.headerBytes) };
}
