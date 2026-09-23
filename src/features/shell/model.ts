// View model of the reader shell.
//
// ErrorCode, FindingKind and PageSize mirror crates/ipc_contract (MVP-02). Once MVP-06 wires the
// shell to real IPC, replace these with the generated types in src/ipc/generated/contract.ts.

export type ErrorCode =
  | "unknownDocument"
  | "invalidArgument"
  | "cancelled"
  | "notPdf"
  | "corrupted"
  | "encrypted"
  | "unreadable"
  | "tooLarge"
  | "limitExceeded"
  | "workerCrashed"
  | "workerTimeout"
  | "protocolViolation"
  | "internal";

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
] as const;

export type FindingKind = (typeof FINDING_KINDS)[number];

export type SecurityFinding = { kind: FindingKind; count: number };

export type PageSize = { widthPt: number; heightPt: number };

export type OutlineEntry = { title: string; depth: number; pageIndex: number };

export type ShellDocument = {
  /** File name only, never a path. */
  displayName: string;
  pages: PageSize[];
  outline: OutlineEntry[];
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

/** Next zoom level in the given direction; fit modes step from 100%. */
export function stepZoom(zoom: Zoom, direction: 1 | -1): number {
  const current = typeof zoom === "number" ? zoom : 100;
  if (direction === 1) {
    return ZOOM_LEVELS.find((level) => level > current) ?? ZOOM_LEVELS[ZOOM_LEVELS.length - 1]!;
  }
  return [...ZOOM_LEVELS].reverse().find((level) => level < current) ?? ZOOM_LEVELS[0]!;
}

export function rotate(rotation: Rotation, direction: 1 | -1): Rotation {
  return (((rotation + direction * 90) % 360) + 360) % 360 as Rotation;
}
