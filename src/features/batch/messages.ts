// What the user is told about one file of a batch (B3-04).

import { strings } from "@/i18n/zh-TW";
import type { BatchOutcome, ErrorCode } from "@/ipc/generated/contract";

const t = strings.batch;

/** Why a file failed: the text for its code here, else the general one for that code. */
export function reasonFor(code: ErrorCode): string {
  return t.reasons[code] ?? (strings.error.messages[code] || strings.error.messages.internal);
}

/** How a file went, or that it is still waiting. */
export function describeOutcome(outcome: BatchOutcome | null): string {
  if (outcome === null) return t.waiting;
  switch (outcome.kind) {
    case "done":
      return t.done(outcome.output, outcome.bates);
    case "failed":
      return t.failedFile(reasonFor(outcome.error));
    case "skipped":
      return t.skipped;
  }
}
