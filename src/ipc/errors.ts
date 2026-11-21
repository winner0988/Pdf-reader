// What the user is told when a command fails (#634): the message for its error code, never the
// developer-facing `message`.

import type { ErrorCode, IpcError } from "@/ipc/generated/contract";
import { strings } from "@/i18n/zh-TW";

/** The code of a rejected command, or `internal` when the value is not an `IpcError` with a known code. */
export function ipcErrorCode(error: unknown): ErrorCode {
  const code = (error as Partial<IpcError> | null | undefined)?.code;
  return typeof code === "string" && Object.hasOwn(strings.error.messages, code)
    ? (code as ErrorCode)
    : "internal";
}

/** The sentence to show for a rejected command. */
export function ipcErrorMessage(error: unknown): string {
  return strings.error.messages[ipcErrorCode(error)];
}
