import { LockKeyhole, ScanText } from "lucide-react";

import { formatZoom } from "@/features/shell/format";
import type { Zoom } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

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
};

export function StatusBar({ document, hoverTarget, hint, restriction, ocr, ocrNote }: StatusBarProps) {
  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t bg-muted/60 px-3 text-xs text-muted-foreground">
      {document && (
        <>
          <span className="truncate">{document.displayName}</span>
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
