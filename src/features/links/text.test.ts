import { describe, expect, it } from "vitest";

import { HOVER_MAX_CHARS, hasHiddenCharacters, linkHoverText, revealHidden } from "@/features/links/text";
import { strings } from "@/i18n/zh-TW";
import type { BlockedAction } from "@/ipc/generated/contract";

describe("revealHidden", () => {
  it("writes out what would otherwise be invisible or reorder the text", () => {
    // U+202E would make ".../fdp.exe" display as ".../exe.pdf".
    expect(revealHidden("https://example.invalid/‮fdp.exe")).toBe("https://example.invalid/[U+202E]fdp.exe");
    expect(revealHidden("a\u0000b\u007Fc​d﻿e⁦f؜g")).toBe(
      "a[U+0000]b[U+007F]c[U+200B]d[U+FEFF]e[U+2066]f[U+061C]g",
    );
    expect(revealHidden("line\nbreak\ttab")).toBe("line[U+000A]break[U+0009]tab");
  });

  it("leaves ordinary text alone, including other scripts", () => {
    const text = "https://аpple.example.invalid/路徑?q=1#x";
    expect(revealHidden(text)).toBe(text);
    expect(hasHiddenCharacters(text)).toBe(false);
    expect(hasHiddenCharacters("x‮y")).toBe(true);
  });
});

describe("linkHoverText", () => {
  it("says which page an internal link goes to", () => {
    expect(linkHoverText({ kind: "page", pageIndex: 2, x: null, y: null })).toBe("前往第 3 頁");
  });

  it("shows a web link with its hidden characters marked, cut in the status bar only", () => {
    expect(linkHoverText({ kind: "uri", uri: "https://example.invalid/‮fdp.exe" })).toBe(
      "https://example.invalid/[U+202E]fdp.exe",
    );
    const long = linkHoverText({ kind: "uri", uri: `https://example.invalid/${"a".repeat(10_000)}` });
    expect(long).toHaveLength(HOVER_MAX_CHARS + 1);
    expect(long.endsWith("…")).toBe(true);
  });

  it("says why a blocked link is blocked", () => {
    const reasons: Record<BlockedAction, string> = {
      javaScript: "腳本連結",
      localFile: "本機檔案連結",
      networkShare: "網路共用路徑",
      other: "不支援的連結類型",
      launch: "啟動外部程式",
      remoteGoTo: "開啟其他文件",
      embeddedGoTo: "開啟內嵌文件",
      submitForm: "表單傳送",
      importData: "匯入外部資料",
    };
    for (const [action, reason] of Object.entries(reasons) as [BlockedAction, string][]) {
      expect(linkHoverText({ kind: "blocked", action, target: null })).toBe(`已封鎖：${reason}`);
      expect(strings.links.blocked[action].description).toMatch(/。$/);
    }
  });
});
