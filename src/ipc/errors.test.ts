import { describe, expect, it } from "vitest";

import { strings } from "@/i18n";
import { ipcErrorCode, ipcErrorMessage } from "@/ipc/errors";

describe("ipcErrorCode and ipcErrorMessage (#634)", () => {
  it("maps known error code to its message and code", () => {
    const error = { code: "workerCrashed", message: "x" };
    expect(ipcErrorCode(error)).toBe("workerCrashed");
    expect(ipcErrorMessage(error)).toBe(strings.error.messages.workerCrashed);
  });

  it("maps unknown, prototype, non-string, or non-IpcError values to internal", () => {
    const cases = [
      { code: "notACode", message: "x" },
      { code: "toString" },
      { code: 5 },
      new Error("boom"),
      "text",
      null,
      undefined,
    ];

    for (const c of cases) {
      expect(ipcErrorCode(c)).toBe("internal");
      expect(ipcErrorMessage(c)).toBe(strings.error.messages.internal);
    }
  });
});
