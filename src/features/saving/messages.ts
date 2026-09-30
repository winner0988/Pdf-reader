// What the user is told when saving fails (B2-02): why, that the changes are still there, and
// what to try.

import type { ErrorCode, IpcError } from "@/ipc/generated/contract";
import { strings } from "@/i18n/zh-TW";

/** Failures where saving a copy elsewhere may still work. */
const SAVE_AS_HELPS: ErrorCode[] = ["readOnly", "fileInUse", "changedOnDisk", "unwritable", "diskFull"];

export function saveErrorCode(failure: unknown): ErrorCode {
  const code = (failure as Partial<IpcError> | null)?.code;
  return typeof code === "string" && code in strings.error.messages ? code : "internal";
}

/** The reason, then that nothing was lost, then (when it can help) to save a copy instead. */
export function saveFailure(failure: unknown): string {
  const code = saveErrorCode(failure);
  const advice = SAVE_AS_HELPS.includes(code) ? ` ${strings.saving.trySaveAs}` : "";
  return `${strings.error.messages[code]} ${strings.saving.keptChanges}${advice}`;
}

/** Whether "save as" is worth offering after `failure`. */
export function saveAsHelps(failure: unknown): boolean {
  return SAVE_AS_HELPS.includes(saveErrorCode(failure));
}
