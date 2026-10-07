// What the signatures of a document add up to (B2-14, docs/ux/screen-map.md).

import { strings } from "@/i18n/zh-TW";
import type { SignatureInfo, SignatureReport } from "@/ipc/generated/contract";

/**
 * How a signature is told to the user. A valid signature is `valid` only if its signer is
 * trusted: otherwise "valid" would be taken for more than it says.
 */
export type Category = "valid" | "unconfirmed" | "changed" | "invalid" | "unverifiable";

export function categoryOf(signature: SignatureInfo): Category {
  switch (signature.status) {
    case "valid":
      return signature.signerTrusted ? "valid" : "unconfirmed";
    case "changedAfterSigning":
      return "changed";
    case "invalid":
      return "invalid";
    case "unverifiable":
      return "unverifiable";
  }
}

/** Worst first: what the banner follows when the signatures of a document differ. */
const WORST_FIRST: Category[] = ["invalid", "changed", "unverifiable", "unconfirmed", "valid"];

export function worstCategory(report: SignatureReport): Category {
  const found = new Set(report.signatures.map(categoryOf));
  return WORST_FIRST.find((category) => found.has(category)) ?? "valid";
}

/** The banner's sentence: the one signature, or how many there are of each kind. */
export function bannerSummary(report: SignatureReport): string {
  const t = strings.signatures;
  const [only, ...others] = report.signatures;
  let text: string;
  if (only !== undefined && others.length === 0) {
    text = t.summaryOne[categoryOf(only)];
  } else {
    const counts = new Map<Category, number>();
    for (const signature of report.signatures) {
      const category = categoryOf(signature);
      counts.set(category, (counts.get(category) ?? 0) + 1);
    }
    const parts = WORST_FIRST.filter((category) => counts.has(category)).map((category) =>
      t.summaryPart[category](counts.get(category)!),
    );
    text = t.summaryMany(report.signatures.length, parts);
  }
  return report.truncated ? `${text}${t.truncated(report.signatures.length)}` : text;
}
