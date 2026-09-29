import { LockKeyhole } from "lucide-react";

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
};

export function StatusBar({ document, hoverTarget, hint, restriction }: StatusBarProps) {
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
