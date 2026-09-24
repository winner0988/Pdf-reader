// View model of the reader shell. Contract types come from crates/ipc_contract through the
// generated src/ipc/generated/contract.ts.

import type { OutlineView } from "@/features/outline/tree";
import type { DocumentId, ErrorCode, FindingKind, PageSize, SecurityFinding } from "@/ipc/generated/contract";

export type { ErrorCode, FindingKind, PageSize, SecurityFinding };

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


export type ShellDocument = {
  /** Main-process id for rendering; absent for demo data. */
  doc?: DocumentId;
  /** File name only, never a path. */
  displayName: string;
  pages: PageSize[];
  /** Demo data only; real documents load their outline separately (ReaderShell's `outline`). */
  outline?: OutlineView;
  findings: SecurityFinding[];
};

export type ShellState =
  | { kind: "empty" }
  | { kind: "loading"; displayName: string }
  | { kind: "error"; code: ErrorCode; displayName?: string }
  | { kind: "open"; document: ShellDocument };

/** Errors after which retrying can succeed (docs/ux/screen-map.md, section 2). */
export const RETRYABLE_ERRORS: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
  "workerCrashed",
  "workerTimeout",
  "unreadable",
]);

export type Rotation = 0 | 90 | 180 | 270;

/** Zoom is either a percentage or a fit mode that follows the window size. */
export type Zoom = number | "fitWidth" | "fitPage";

export const ZOOM_LEVELS = [25, 33, 50, 67, 75, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500, 800];

/** Next zoom level in the given direction; fit modes step from the percentage they show. */
export function stepZoom(zoom: Zoom, direction: 1 | -1, fitPercent = 100): number {
  const current = typeof zoom === "number" ? zoom : fitPercent;
  if (direction === 1) {
    return ZOOM_LEVELS.find((level) => level > current) ?? ZOOM_LEVELS[ZOOM_LEVELS.length - 1]!;
  }
  return [...ZOOM_LEVELS].reverse().find((level) => level < current) ?? ZOOM_LEVELS[0]!;
}

export function rotate(rotation: Rotation, direction: 1 | -1): Rotation {
  return (((rotation + direction * 90) % 360) + 360) % 360 as Rotation;
}
