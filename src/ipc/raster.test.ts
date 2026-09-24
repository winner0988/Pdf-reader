import { describe, expect, it } from "vitest";

import { LIMITS } from "@/ipc/generated/contract";
import { RasterFormatError, decodeRaster } from "@/ipc/raster";

// Same bytes as TEST_VECTOR in crates/ipc_contract/src/raster.rs.
const TEST_VECTOR = [
  0x50, 0x44, 0x46, 0x52, 1, 0, 0, 0, // "PDFR", format 1, reserved 0
  2, 0, 0, 0, 1, 0, 0, 0, // width 2, height 1
  255, 0, 0, 255, 0, 0, 255, 255, // red, blue
];

function bufferOf(bytes: number[]): ArrayBuffer {
  return new Uint8Array(bytes).buffer;
}

function header(width: number, height: number): number[] {
  const view = new DataView(new ArrayBuffer(16));
  "PDFR".split("").forEach((char, i) => view.setUint8(i, char.charCodeAt(0)));
  view.setUint16(4, 1, true);
  view.setUint32(8, width, true);
  view.setUint32(12, height, true);
  return Array.from(new Uint8Array(view.buffer));
}

describe("decodeRaster", () => {
  it("decodes the shared test vector", () => {
    const raster = decodeRaster(bufferOf(TEST_VECTOR));

    expect(raster.width).toBe(2);
    expect(raster.height).toBe(1);
    expect(Array.from(raster.pixels)).toEqual([255, 0, 0, 255, 0, 0, 255, 255]);
  });

  it("returns a view into the response instead of copying it", () => {
    const buffer = bufferOf(TEST_VECTOR);

    expect(decodeRaster(buffer).pixels.buffer).toBe(buffer);
  });

  it.each([
    ["a truncated header", TEST_VECTOR.slice(0, 10)],
    ["a bad magic", [0, ...TEST_VECTOR.slice(1)]],
    ["an unknown format", [...TEST_VECTOR.slice(0, 4), 2, ...TEST_VECTOR.slice(5)]],
    ["a non-zero reserved field", [...TEST_VECTOR.slice(0, 6), 1, ...TEST_VECTOR.slice(7)]],
    ["missing pixels", TEST_VECTOR.slice(0, -1)],
    ["extra pixels", [...TEST_VECTOR, 0]],
    ["a zero width", [...header(0, 1)]],
    ["a side above the limit", [...header(LIMITS.maxRasterSidePx + 1, 1)]],
    ["an area above the limit", [...header(LIMITS.maxRasterSidePx, LIMITS.maxRasterSidePx)]],
  ])("rejects %s", (_, bytes) => {
    expect(() => decodeRaster(bufferOf(bytes))).toThrow(RasterFormatError);
  });
});
