import { describe, expect, it } from "vitest";

import { bannerSummary, formatZoom } from "@/features/shell/format";

describe("bannerSummary", () => {
  it("totals the counts and lists kinds in display order", () => {
    expect(
      bannerSummary([
        { kind: "remoteFileSpec", count: 1 },
        { kind: "javaScript", count: 2 },
      ]),
    ).toBe("已封鎖此文件中的 3 項內容：JavaScript 腳本、遠端資源引用。這些內容不會執行。");
  });

  it("names at most three kinds and adds 等 when there are more", () => {
    expect(
      bannerSummary([
        { kind: "xfa", count: 1 },
        { kind: "launch", count: 1 },
        { kind: "openAction", count: 1 },
        { kind: "javaScript", count: 1 },
      ]),
    ).toBe("已封鎖此文件中的 4 項內容：JavaScript 腳本、開檔自動動作、啟動外部程式等。這些內容不會執行。");
  });
});

describe("formatZoom", () => {
  it("formats percentages and fit modes", () => {
    expect(formatZoom(125)).toBe("125%");
    expect(formatZoom("fitWidth")).toBe("符合寬度");
    expect(formatZoom("fitPage")).toBe("符合頁面");
  });
});
