// What the blocked-content banner and its details panel say (MVP-11, docs/ux/screen-map.md,
// sections 1 and 5). The findings come from the worker's scan when the document opens.

import { strings } from "@/i18n/zh-TW";
import type { FindingKind, SecurityFinding } from "@/ipc/generated/contract";

/** Display order of blocked-content kinds (docs/ux/screen-map.md, section 8). */
export const FINDING_KINDS = [
  "javaScript",
  "openAction",
  "additionalActions",
  "launch",
  "submitForm",
  "importData",
  "remoteGoTo",
  "embeddedGoTo",
  "remoteFileSpec",
  "uncReference",
  "xfa",
  "richMedia",
  "embeddedFile",
] as const satisfies readonly FindingKind[];

// Compile-time check: every kind the contract knows has a place in the display order.
type Assert<T extends true> = T;
export type AllFindingKindsListed = Assert<
  [Exclude<FindingKind, (typeof FINDING_KINDS)[number]>] extends [never] ? true : false
>;

/** Kinds that can leak something about the user just by being followed: shown with emphasis. */
export const LEAKY_KINDS: ReadonlySet<FindingKind> = new Set<FindingKind>(["uncReference"]);

/** The findings in display order, without empty ones. */
export function orderedFindings(findings: readonly SecurityFinding[]): SecurityFinding[] {
  return findings
    .filter((finding) => finding.count > 0)
    .sort((a, b) => FINDING_KINDS.indexOf(a.kind) - FINDING_KINDS.indexOf(b.kind));
}

/** Whether the banner is shown: something was blocked, or the scan could not finish. */
export function hasBannerContent(findings: readonly SecurityFinding[], scanComplete: boolean): boolean {
  return orderedFindings(findings).length > 0 || !scanComplete;
}

/**
 * The banner sentence: how many kinds were blocked and up to three of their names. Kinds, not
 * the sum of the counts, because the kinds overlap: a script run by a page event is both a
 * script and an event action.
 */
export function bannerSummary(findings: readonly SecurityFinding[], scanComplete = true): string {
  const ordered = orderedFindings(findings);
  if (ordered.length === 0) return scanComplete ? "" : strings.banner.scanIncomplete;
  const names = ordered.slice(0, 3).map((finding) => strings.findings[finding.kind].name);
  return strings.banner.summary(ordered.length, names, ordered.length > 3);
}
