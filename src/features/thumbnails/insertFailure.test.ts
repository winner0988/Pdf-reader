import { describe, expect, it } from "vitest";

import { insertFailureMessage } from "@/features/thumbnails/insertFailure";
import { strings } from "@/i18n/zh-TW";

describe("insertFailureMessage", () => {
  it("maps notAllowed to sourceNotAllowed", () => {
    expect(insertFailureMessage({ code: "notAllowed", message: "x" })).toBe(strings.pages.sourceNotAllowed);
  });

  it("maps limitExceeded and tooLarge to sourceTooLarge", () => {
    expect(insertFailureMessage({ code: "limitExceeded", message: "x" })).toBe(strings.pages.sourceTooLarge);
    expect(insertFailureMessage({ code: "tooLarge", message: "x" })).toBe(strings.pages.sourceTooLarge);
  });

  it("maps failed, Error instance, and null to sourceFailed", () => {
    expect(insertFailureMessage({ code: "failed", message: "x" })).toBe(strings.pages.sourceFailed);
    expect(insertFailureMessage(new Error("x"))).toBe(strings.pages.sourceFailed);
    expect(insertFailureMessage(null)).toBe(strings.pages.sourceFailed);
  });
});
