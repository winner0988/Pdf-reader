import { strings } from "@/i18n/zh-TW";

/** What a language is called: its name where the app knows it, with its code; otherwise the code. */
export function languageLabel(code: string): string {
  const name = strings.ocr.languages[code];
  return name ? `${name}（${code}）` : code;
}
