import { describe, expect, it } from "vitest";

import { bannerSummary, hasBannerContent, orderedFindings } from "@/features/security-banner/summary";
import { strings } from "@/i18n/zh-TW";

describe("bannerSummary", () => {
  it("counts the kinds and names them in display order", () => {
    expect(
      bannerSummary([
        { kind: "remoteFileSpec", count: 1 },
        { kind: "javaScript", count: 2 },
      ]),
    ).toBe("已封鎖此文件中的 2 項內容：JavaScript 腳本、遠端資源引用。這些內容不會執行。");
  });

  it("does not add up overlapping counts: a script run by a page event is one thing", () => {
    expect(
      bannerSummary([
        { kind: "javaScript", count: 2 },
        { kind: "additionalActions", count: 2 },
      ]),
    ).toBe("已封鎖此文件中的 2 項內容：JavaScript 腳本、事件觸發動作。這些內容不會執行。");
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

  it("says the scan did not finish when that is all there is to say", () => {
    expect(bannerSummary([], false)).toBe(strings.banner.scanIncomplete);
    expect(bannerSummary([{ kind: "xfa", count: 1 }], false)).toContain("XFA 動態表單");
  });
});

describe("hasBannerContent", () => {
  it("shows the banner for findings or an unfinished scan, not for a clean document", () => {
    expect(hasBannerContent([{ kind: "embeddedFile", count: 1 }], true)).toBe(true);
    expect(hasBannerContent([], false)).toBe(true);
    expect(hasBannerContent([], true)).toBe(false);
    expect(hasBannerContent([{ kind: "javaScript", count: 0 }], true)).toBe(false);
  });
});

describe("orderedFindings", () => {
  it("sorts by the display order and drops empty kinds", () => {
    expect(
      orderedFindings([
        { kind: "embeddedFile", count: 1 },
        { kind: "uncReference", count: 3 },
        { kind: "launch", count: 0 },
        { kind: "openAction", count: 1 },
      ]).map((finding) => finding.kind),
    ).toEqual(["openAction", "uncReference", "embeddedFile"]);
  });
});
