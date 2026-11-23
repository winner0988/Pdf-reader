// Interface strings for PDF Reader (ADR 0029, #683).
// Strings are resolved synchronously on module load based on window.__PDF_READER_LANGUAGE__.
import { strings as zhTwStrings, type Strings } from "./zh-TW";
import { strings as enStrings } from "./en";

export type { Strings };

declare global {
  interface Window {
    __PDF_READER_LANGUAGE__?: string;
    __PDF_READER_SYSTEM_LANGUAGE__?: string;
  }
}

function resolveStartupLanguage(): "zh-TW" | "en" {
  if (typeof window !== "undefined" && window.__PDF_READER_LANGUAGE__ === "en") {
    return "en";
  }
  return "zh-TW";
}

export const currentLanguage: "zh-TW" | "en" = resolveStartupLanguage();

export const strings: Strings = currentLanguage === "en" ? enStrings : zhTwStrings;

if (typeof document !== "undefined") {
  document.documentElement.lang = currentLanguage === "en" ? "en" : "zh-Hant-TW";
}

export function getSystemLanguage(): "zh-TW" | "en" {
  if (typeof window !== "undefined" && window.__PDF_READER_SYSTEM_LANGUAGE__ === "en") {
    return "en";
  }
  return "zh-TW";
}
