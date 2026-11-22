import type { PageSize } from "@/ipc/generated/contract";
import type { RasterImage } from "@/ipc/raster";

/** Target width in pixels to render both pages at so differences align horizontally. */
export const COMPARE_WIDTH_PX = 1000;

/** Maximum allowable height in pixels to avoid excessive memory usage for very tall pages. */
export const MAX_COMPARE_HEIGHT_PX = 3000;

/** Color channel difference threshold above which two pixels are considered different. */
export const PIXEL_THRESHOLD = 48;

/** Side length in pixels of each comparison grid cell. */
export const CELL_PX = 8;

/** Maximum distance in cells between two regions before they are merged into one. */
export const MERGE_GAP_CELLS = 2;

/** Maximum number of difference regions reported to keep DOM and memory bounded. */
export const MAX_REGIONS = 100;

export type Rect = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type PagePair = {
  a: number | null;
  b: number | null;
};

export type DiffResult = {
  regions: Rect[];
  truncated: boolean;
};

/** Aligns two documents by page index, yielding null on the shorter side for extra pages. */
export function pagePairs(countA: number, countB: number): PagePair[] {
  const total = Math.max(countA, countB);
  const pairs: PagePair[] = [];
  for (let i = 0; i < total; i++) {
    pairs.push({
      a: i < countA ? i : null,
      b: i < countB ? i : null,
    });
  }
  return pairs;
}

/** Computes the scale factor to render a page at COMPARE_WIDTH_PX, capping height at MAX_COMPARE_HEIGHT_PX. */
export function compareScale(size: PageSize): number {
  const scale = COMPARE_WIDTH_PX / size.widthPt;
  if (size.heightPt * scale > MAX_COMPARE_HEIGHT_PX) {
    return MAX_COMPARE_HEIGHT_PX / size.heightPt;
  }
  return scale;
}

type CellBox = {
  minC: number;
  maxC: number;
  minR: number;
  maxR: number;
};

/** Compares two rendered raster images and returns the bounding boxes of differing areas. */
export function diffRegions(a: RasterImage, b: RasterImage): DiffResult {
  const minW = Math.min(a.width, b.width);
  const minH = Math.min(a.height, b.height);
  const maxW = Math.max(a.width, b.width);
  const maxH = Math.max(a.height, b.height);

  const cols = Math.ceil(minW / CELL_PX);
  const rows = Math.ceil(minH / CELL_PX);

  // Identify dirty cells in the overlapping region
  const dirty = new Uint8Array(rows * cols);

  for (let r = 0; r < rows; r++) {
    const startY = r * CELL_PX;
    const endY = Math.min(minH, (r + 1) * CELL_PX);

    for (let c = 0; c < cols; c++) {
      const startX = c * CELL_PX;
      const endX = Math.min(minW, (c + 1) * CELL_PX);

      let cellDiff = false;
      for (let y = startY; y < endY; y++) {
        const rowOffsetA = (y * a.width + startX) * 4;
        const rowOffsetB = (y * b.width + startX) * 4;
        let idxA = rowOffsetA;
        let idxB = rowOffsetB;

        for (let x = startX; x < endX; x++) {
          const paR = a.pixels[idxA];
          const pbR = b.pixels[idxB];
          const paG = a.pixels[idxA + 1];
          const pbG = b.pixels[idxB + 1];
          const paB = a.pixels[idxA + 2];
          const pbB = b.pixels[idxB + 2];

          if (
            paR !== undefined &&
            pbR !== undefined &&
            paG !== undefined &&
            pbG !== undefined &&
            paB !== undefined &&
            pbB !== undefined
          ) {
            const diffR = Math.abs(paR - pbR);
            const diffG = Math.abs(paG - pbG);
            const diffB = Math.abs(paB - pbB);

            if (diffR > PIXEL_THRESHOLD || diffG > PIXEL_THRESHOLD || diffB > PIXEL_THRESHOLD) {
              cellDiff = true;
              break;
            }
          }
          idxA += 4;
          idxB += 4;
        }
        if (cellDiff) break;
      }

      if (cellDiff) {
        dirty[r * cols + c] = 1;
      }
    }
  }

  // Find 8-connected components of dirty cells
  const visited = new Uint8Array(rows * cols);
  const components: CellBox[] = [];

  const queue: number[] = [];

  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const idx = r * cols + c;
      if (dirty[idx] === 0 || visited[idx] === 1) continue;

      visited[idx] = 1;
      queue.push(idx);

      let minC = c;
      let maxC = c;
      let minR = r;
      let maxR = r;

      let qHead = 0;
      while (qHead < queue.length) {
        const curr = queue[qHead++];
        if (curr === undefined) break;
        const currR = Math.floor(curr / cols);
        const currC = curr % cols;

        if (currC < minC) minC = currC;
        if (currC > maxC) maxC = currC;
        if (currR < minR) minR = currR;
        if (currR > maxR) maxR = currR;

        for (let dr = -1; dr <= 1; dr++) {
          const nr = currR + dr;
          if (nr < 0 || nr >= rows) continue;
          for (let dc = -1; dc <= 1; dc++) {
            if (dr === 0 && dc === 0) continue;
            const nc = currC + dc;
            if (nc < 0 || nc >= cols) continue;
            const nIdx = nr * cols + nc;
            if (dirty[nIdx] === 1 && visited[nIdx] === 0) {
              visited[nIdx] = 1;
              queue.push(nIdx);
            }
          }
        }
      }
      queue.length = 0;

      components.push({ minC, maxC, minR, maxR });
    }
  }

  // Merge components whose cell bounding boxes are within MERGE_GAP_CELLS in both dimensions
  const boxes = components;
  let merged = true;
  while (merged) {
    merged = false;
    for (let i = 0; i < boxes.length; i++) {
      for (let j = i + 1; j < boxes.length; j++) {
        const b1 = boxes[i];
        const b2 = boxes[j];
        if (!b1 || !b2) continue;

        const gapC =
          b1.maxC < b2.minC ? b2.minC - b1.maxC - 1 : b2.maxC < b1.minC ? b1.minC - b2.maxC - 1 : 0;
        const gapR =
          b1.maxR < b2.minR ? b2.minR - b1.maxR - 1 : b2.maxR < b1.minR ? b1.minR - b2.maxR - 1 : 0;

        if (gapC <= MERGE_GAP_CELLS && gapR <= MERGE_GAP_CELLS) {
          boxes[i] = {
            minC: Math.min(b1.minC, b2.minC),
            maxC: Math.max(b1.maxC, b2.maxC),
            minR: Math.min(b1.minR, b2.minR),
            maxR: Math.max(b1.maxR, b2.maxR),
          };
          boxes.splice(j, 1);
          merged = true;
          break;
        }
      }
      if (merged) break;
    }
  }

  // Convert merged cell boxes to pixel Rects (aligned to cell grid)
  const regions: Rect[] = boxes.map((box) => {
    const x = box.minC * CELL_PX;
    const y = box.minR * CELL_PX;
    const width = (box.maxC - box.minC + 1) * CELL_PX;
    const height = (box.maxR - box.minR + 1) * CELL_PX;
    return { x, y, width, height };
  });

  // Size difference: if width or height diff > 2 px, extra areas count as distinct blocks
  if (maxW - minW > 2) {
    regions.push({
      x: minW,
      y: 0,
      width: maxW - minW,
      height: maxH,
    });
  }

  if (maxH - minH > 2) {
    regions.push({
      x: 0,
      y: minH,
      width: minW,
      height: maxH - minH,
    });
  }

  // Sort regions by y, then by x
  regions.sort((r1, r2) => (r1.y !== r2.y ? r1.y - r2.y : r1.x - r2.x));

  const truncated = regions.length > MAX_REGIONS;
  const finalRegions = truncated ? regions.slice(0, MAX_REGIONS) : regions;

  return {
    regions: finalRegions,
    truncated,
  };
}
