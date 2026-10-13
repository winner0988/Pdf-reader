// Compressing the pictures of a copy (B3-01, docs/architecture/compress-images.md): what the user
// chooses, what is sent to the main process, and what the result is called.

import { strings } from "@/i18n/zh-TW";
import type { CompressArgs, CompressResult, DocumentId, ImageQuality } from "@/ipc/generated/contract";

const t = strings.compress;

/** The qualities offered, best first. */
export const QUALITIES: readonly ImageQuality[] = ["high", "medium", "low"];

/** How long the longer side of a picture may be, in pixels; `null`: pictures are not shrunk. */
export const SIDES: readonly (number | null)[] = [null, 3000, 2000, 1500, 1000];

export type CompressForm = { quality: ImageQuality; maxSide: number | null };

/** What is chosen when the dialog opens. */
export const DEFAULT_FORM: CompressForm = { quality: "medium", maxSide: 2000 };

export function argsOf(doc: DocumentId, form: CompressForm): CompressArgs {
  return { doc, quality: form.quality, maxSide: form.maxSide };
}

const KIB = 1024;
const MIB = KIB * KIB;

/** A size in bytes the way a person says it: kilobytes below a megabyte, else megabytes. */
export function sizeText(bytes: number): string {
  if (bytes < MIB) return t.kilobytes(Math.max(1, Math.round(bytes / KIB)));
  return t.megabytes((bytes / MIB).toFixed(1));
}

/** What became of the pictures, in a sentence or two. */
export function describe(result: CompressResult): string {
  const after = sizeText(result.after);
  if (result.images === 0) return t.result.none(after);
  const parts: string[] = [];
  if (result.changed === 0 || result.after >= result.before) {
    parts.push(t.result.unchanged(after, result.images));
  } else {
    const saved = Math.max(1, Math.round((1 - result.after / result.before) * 100));
    parts.push(t.result.done(sizeText(result.before), after, saved, result.changed, result.images));
  }
  if (result.partial) parts.push(t.result.partial);
  return parts.join("");
}
