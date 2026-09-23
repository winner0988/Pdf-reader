import { formatZoom } from "@/features/shell/format";
import type { Zoom } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

type StatusBarProps = {
  document: { displayName: string; currentPage: number; pageCount: number; zoom: Zoom } | null;
  /** Link target under the pointer (MVP-12). */
  hoverTarget?: string;
};

export function StatusBar({ document, hoverTarget }: StatusBarProps) {
  return (
    <footer className="flex h-7 shrink-0 items-center gap-4 border-t bg-muted/60 px-3 text-xs text-muted-foreground">
      {document && (
        <>
          <span className="truncate">{document.displayName}</span>
          <span className="ml-auto truncate">{hoverTarget}</span>
          <span className="shrink-0">
            {strings.statusBar.pageStatus(document.currentPage, document.pageCount, formatZoom(document.zoom))}
          </span>
        </>
      )}
    </footer>
  );
}
