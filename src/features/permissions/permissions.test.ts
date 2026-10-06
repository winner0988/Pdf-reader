import { describe, expect, it } from "vitest";

import { ALL_PERMISSIONS, restrictionSummary } from "@/features/permissions/permissions";

describe("restrictionSummary", () => {
  it("says nothing when everything is allowed", () => {
    expect(restrictionSummary(ALL_PERMISSIONS)).toBeNull();
  });

  it("lists what the author forbids", () => {
    expect(restrictionSummary({ copy: false, print: false, printHighQuality: false, modify: true, assemble: true, annotate: true })).toBe(
      "已限制：不可複製、不可列印",
    );
    expect(restrictionSummary({ ...ALL_PERMISSIONS, copy: false })).toBe("已限制：不可複製");
  });

  it("mentions low-resolution printing only when printing is allowed", () => {
    expect(restrictionSummary({ ...ALL_PERMISSIONS, printHighQuality: false })).toBe("已限制：只能低解析度列印");
    expect(restrictionSummary({ copy: true, print: false, printHighQuality: true, modify: true, assemble: true, annotate: true })).toBe("已限制：不可列印");
  });
});
