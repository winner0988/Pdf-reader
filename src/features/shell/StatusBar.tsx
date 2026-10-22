import { LockKeyhole, MonitorOff, ScanText, ShieldCheck, TriangleAlert } from "lucide-react";

import { formatZoom } from "@/features/shell/format";
import type { Zoom } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";
import type { CaptureProtection } from "@/ipc/generated/contract";

type StatusBarProps = {
  document: { displayName: string; currentPage: number; pageCount: number; zoom: Zoom } | null;
  /** Link target under the pointer (MVP-12). */
  hoverTarget?: string;
  /** A passing message, such as why Ctrl+C did nothing; screen readers announce it. */
  hint?: string;
  /** What the document's author restricts (MVP-19). */
  restriction?: string | null;
  /** Scanned pages are being read (B2-10): how far it is, and a way to stop it. */
  ocr?: { text: string; onStop: () => void };
  /** The page shown has text that was recognised from its picture, and may be wrong (B2-10). */
  ocrNote?: string | null;
  /** The document is marked as sensitive (B3-05): its copied text stays out of the clipboard history. */
  sensitive?: boolean;
  /**
   * Whether the window is kept out of screen captures (B3-13, ADR 0023): while any tab has a
   * sensitive document, so this tab's too, sensitive or not.
   */
  capture?: CaptureProtection;
};

export function StatusBar({
  document,
  hoverTarget,
  hint,
  restriction,
  ocr,
  ocrNote,
  sensitive,
  capture = "off",
}: StatusBarProps) {
  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t bg-muted/60 px-3 text-xs text-muted-foreground">
      {document && (
        <>
          <span className="truncate">{document.displayName}</span>
          {sensitive && (
            <span className="flex shrink-0 items-center gap-1" title={strings.sensitive.badgeNote}>
              <ShieldCheck aria-hidden className="size-3" />
              {strings.sensitive.badge}
            </span>
          )}
          {capture === "on" && (
            <span className="flex shrink-0 items-center gap-1" title={strings.sensitive.captureOnNote}>
              <MonitorOff aria-hidden className="size-3" />
              {strings.sensitive.captureOn}
            </span>
          )}
          {capture === "failed" && (
            <span className="flex shrink-0 items-center gap-1 text-destructive" title={strings.sensitive.captureFailedNote}>
              <TriangleAlert aria-hidden className="size-3" />
              {strings.sensitive.captureFailed}
            </span>
          )}
          {restriction && (
            <span className="flex shrink-0 items-center gap-1">
              <LockKeyhole aria-hidden className="size-3" />
              {restriction}
            </span>
          )}
          {ocr && (
            <span className="flex shrink-0 items-center gap-2">
              <span role="status">{ocr.text}</span>
              <button
                type="button"
                aria-label={strings.ocr.stopLabel}
                className="underline underline-offset-2 hover:text-foreground"
                onClick={ocr.onStop}
              >
                {strings.ocr.stop}
              </button>
            </span>
          )}
          {ocrNote && (
            <span data-testid="ocr-note" className="flex shrink-0 items-center gap-1">
              <ScanText aria-hidden className="size-3" />
              {ocrNote}
            </span>
          )}
          {hoverTarget ? (
            <span className="ml-auto truncate">{hoverTarget}</span>
          ) : (
            <span role="status" className="ml-auto truncate">
              {hint}
            </span>
          )}
          <span className="shrink-0">
            {strings.statusBar.pageStatus(document.currentPage, document.pageCount, formatZoom(document.zoom))}
          </span>
        </>
      )}
    </footer>
  );
}
