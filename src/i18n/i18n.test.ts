import { describe, expect, it } from "vitest";

import { strings as enStrings } from "./en";
import { strings as zhTwStrings } from "./zh-TW";
import { currentLanguage, getSystemLanguage } from "./index";

const CHINESE_REGEX = /[\u4e00-\u9fa5]/;

const CHINESE_WHITELIST = new Set([
  "settings.languageZhTw",
  "settings.languageTitle",
]);

function callStringFunction(fn: (...args: unknown[]) => unknown, path: string): string[] {
  if (path === "signatures.summaryMany") {
    return [
      fn(1, ["Signature 1"]) as string,
      fn(2, ["Signature 1", "Signature 2"]) as string,
    ];
  }
  if (path === "banner.summary") {
    return [
      fn(1, ["JavaScript"], false) as string,
      fn(2, ["JavaScript", "Launch"], true) as string,
    ];
  }
  if (path === "batch.done") {
    return [
      fn("output.pdf", null) as string,
      fn("output.pdf", { first: "0001", last: "0010" }) as string,
    ];
  }
  if (path === "settings.languageSystem") {
    return [fn("English") as string];
  }
  const results: string[] = [];
  try {
    const r1 = fn(1, 1, 1, 1);
    if (typeof r1 === "string") results.push(r1);
  } catch {
    // Expected when parameter count or type is incompatible.
  }
  try {
    const r2 = fn(2, 2, 2, 2);
    if (typeof r2 === "string") results.push(r2);
  } catch {
    // Expected when parameter count or type is incompatible.
  }
  try {
    const r3 = fn("test", "test", "test", "test");
    if (typeof r3 === "string") results.push(r3);
  } catch {
    // Expected when parameter count or type is incompatible.
  }
  return results;
}

function collectStrings(
  obj: unknown,
  path = "",
  out: { path: string; value: string }[] = [],
): { path: string; value: string }[] {
  if (typeof obj === "function") {
    const results = callStringFunction(obj as (...args: unknown[]) => unknown, path);
    for (const res of results) {
      out.push({ path, value: res });
    }
  } else if (typeof obj === "string") {
    out.push({ path, value: obj });
  } else if (Array.isArray(obj)) {
    obj.forEach((item, index) => {
      collectStrings(item, `${path}[${index}]`, out);
    });
  } else if (typeof obj === "object" && obj !== null) {
    for (const [key, value] of Object.entries(obj)) {
      collectStrings(value, path ? `${path}.${key}` : key, out);
    }
  }
  return out;
}

function collectStructurePaths(obj: unknown, path = "", out: string[] = []): string[] {
  if (typeof obj === "object" && obj !== null) {
    if (Array.isArray(obj)) {
      out.push(path);
    } else {
      for (const [key, value] of Object.entries(obj)) {
        collectStructurePaths(value, path ? `${path}.${key}` : key, out);
      }
    }
  } else {
    out.push(path);
  }
  return out;
}

describe("i18n string tables (ADR 0029, #683)", () => {
  it("en strings have no untranslated Chinese characters outside whitelist", () => {
    const items = collectStrings(enStrings);
    const violating = items.filter(
      (item) => !CHINESE_WHITELIST.has(item.path) && CHINESE_REGEX.test(item.value),
    );
    expect(violating).toEqual([]);
  });

  it("en strings have no empty strings except cancelled error", () => {
    const items = collectStrings(enStrings);
    const empty = items.filter(
      (item) => item.path !== "error.messages.cancelled" && item.value.trim().length === 0,
    );
    expect(empty).toEqual([]);
  });

  it("zh-TW and en have the exact same structure", () => {
    const zhKeys = collectStructurePaths(zhTwStrings);
    const enKeys = collectStructurePaths(enStrings);
    expect(new Set(enKeys)).toEqual(new Set(zhKeys));
  });

  it("defaults to zh-TW when window.__PDF_READER_LANGUAGE__ is not set", () => {
    expect(currentLanguage).toBe("zh-TW");
    expect(getSystemLanguage()).toBe("zh-TW");
    expect(document.documentElement.lang).toBe("zh-Hant-TW");
  });
});
