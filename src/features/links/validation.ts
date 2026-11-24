import { strings } from "@/i18n";

export function validateLinkUri(uri: string): string | null {
  const trimmed = uri.trim();
  if (!trimmed) {
    return strings.links.uriError;
  }
  if (/\s/.test(trimmed)) {
    return strings.links.uriError;
  }
  const lower = trimmed.toLowerCase();
  if (
    !lower.startsWith("http://") &&
    !lower.startsWith("https://") &&
    !lower.startsWith("mailto:")
  ) {
    return strings.links.uriError;
  }
  return null;
}

export function validateLinkPage(pageStr: string, pageCount: number): string | null {
  if (pageStr !== pageStr.trim()) {
    return strings.links.pageNumberError(pageCount);
  }
  const num = parseInt(pageStr, 10);
  if (isNaN(num) || num < 1 || num > pageCount || String(num) !== pageStr) {
    return strings.links.pageNumberError(pageCount);
  }
  return null;
}
