import { describe, expect, it } from "vitest";

import { formatZoom } from "@/features/shell/format";

describe("formatZoom", () => {
  it("formats percentages and fit modes", () => {
    expect(formatZoom(125)).toBe("125%");
    expect(formatZoom("fitWidth")).toBe("符合寬度");
    expect(formatZoom("fitPage")).toBe("符合頁面");
  });
});
