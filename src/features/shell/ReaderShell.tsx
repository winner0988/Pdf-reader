import { useRef, useState } from "react";

import { TooltipProvider } from "@/components/ui/tooltip";
import { AboutDialog, ShortcutsDialog } from "@/features/shell/dialogs";
import { rotate, stepZoom, type Rotation, type ShellState, type Zoom } from "@/features/shell/model";
import { SearchBar } from "@/features/shell/SearchBar";
import { SecurityBanner } from "@/features/shell/SecurityBanner";
import { Sidebar } from "@/features/shell/Sidebar";
import { EmptyState, ErrorState, LoadingState } from "@/features/shell/states";
import { StatusBar } from "@/features/shell/StatusBar";
import { Toolbar } from "@/features/shell/Toolbar";
import { useShortcuts } from "@/features/shortcuts/useShortcuts";
import { useTheme } from "@/features/theme/useTheme";
import { DocumentView, type DocumentViewHandle } from "@/features/viewer/DocumentView";
import type { PageRenderer } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";

type ReaderShellProps = {
  state: ShellState;
  onOpen: () => void;
  onClose?: () => void;
  onRetry?: () => void;
  /** Files are being dragged over the window: the canvas shows it is a drop target. */
  dropActive?: boolean;
  /** Renders pages; without it (demo data, tests) pages are placeholders. */
  renderer?: PageRenderer;
  version?: string;
  /** Delay before the loading state appears; tests pass 0. */
  loadingDelayMs?: number;
};

/** Below this width the sidebar floats over the canvas (docs/ux/screen-map.md, section 1). */
const OVERLAY_SIDEBAR_BELOW_PX = 960;

const isNarrowWindow = () => window.innerWidth < OVERLAY_SIDEBAR_BELOW_PX;

/** Region order for F6 / Shift+F6 (docs/ux/screen-map.md, section 7). */
const REGION_ORDER = ["toolbar", "banner", "sidebar", "canvas"];

function focusRegion(direction: 1 | -1) {
  const regions = REGION_ORDER.map((name) =>
    document.querySelector<HTMLElement>(`[data-region="${name}"]`),
  ).filter((element): element is HTMLElement => element !== null);
  if (regions.length === 0) return;
  const current = regions.findIndex((region) => region.contains(document.activeElement));
  const next = regions[(current + direction + regions.length) % regions.length]!;
  const focusable = next.querySelector<HTMLElement>(
    "button:not([disabled]), input, select, [tabindex]:not([tabindex='-1'])",
  );
  (focusable ?? next).focus();
}

export function ReaderShell({
  state,
  onOpen,
  onClose,
  onRetry,
  dropActive = false,
  renderer,
  version = "0.1.0",
  loadingDelayMs,
}: ReaderShellProps) {
  const [theme, setTheme] = useTheme();
  const [sidebarOpen, setSidebarOpen] = useState(() => !isNarrowWindow());
  const [searchOpen, setSearchOpen] = useState(false);
  const [bannerDismissed, setBannerDismissed] = useState(false);
  const [zoom, setZoom] = useState<Zoom>("fitWidth");
  const [rotation, setRotation] = useState<Rotation>(0);
  const [currentPage, setCurrentPage] = useState(1);
  const [dialog, setDialog] = useState<"shortcuts" | "about" | null>(null);
  const pageInputRef = useRef<HTMLInputElement>(null);
  const canvasRef = useRef<HTMLElement>(null);
  const viewRef = useRef<DocumentViewHandle>(null);

  const document_ = state.kind === "open" ? state.document : null;
  const pageCount = document_?.pages.length ?? 0;

  // Per-document view state starts fresh for every newly opened document (nothing is remembered).
  const [viewedDocument, setViewedDocument] = useState(document_);
  if (viewedDocument !== document_) {
    setViewedDocument(document_);
    setZoom("fitWidth");
    setRotation(0);
    setCurrentPage(1);
    setBannerDismissed(false);
    setSearchOpen(false);
  }

  const goToPage = (page: number) => {
    const clamped = Math.min(Math.max(page, 1), pageCount);
    setCurrentPage(clamped);
    viewRef.current?.scrollToPage(clamped);
  };

  const whenOpen = (action: () => void) => () => {
    if (document_) action();
  };

  useShortcuts({
    open: onOpen,
    close: onClose,
    search: whenOpen(() => setSearchOpen(true)),
    zoomIn: whenOpen(() => setZoom((z) => stepZoom(z, 1))),
    zoomOut: whenOpen(() => setZoom((z) => stepZoom(z, -1))),
    fitPage: whenOpen(() => setZoom("fitPage")),
    actualSize: whenOpen(() => setZoom(100)),
    fitWidth: whenOpen(() => setZoom("fitWidth")),
    rotateCw: whenOpen(() => setRotation((r) => rotate(r, 1))),
    rotateCcw: whenOpen(() => setRotation((r) => rotate(r, -1))),
    goToPage: () => {
      if (document_) pageInputRef.current?.focus();
    },
    // Inline rather than through whenOpen: goToPage uses a ref, which must not be touched during render.
    firstPage: () => {
      if (document_) goToPage(1);
    },
    lastPage: () => {
      if (document_) goToPage(pageCount);
    },
    toggleSidebar: () => setSidebarOpen((open) => !open),
    nextRegion: () => focusRegion(1),
    previousRegion: () => focusRegion(-1),
    help: () => setDialog("shortcuts"),
  });

  const findings = document_?.findings ?? [];

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col bg-background text-foreground">
        <Toolbar
          document={document_ ? { pageCount, currentPage, zoom } : null}
          sidebarOpen={sidebarOpen && document_ !== null}
          searchOpen={searchOpen}
          theme={theme}
          pageInputRef={pageInputRef}
          onToggleSidebar={() => setSidebarOpen((open) => !open)}
          onOpen={onOpen}
          onGoToPage={goToPage}
          onZoomIn={() => setZoom((z) => stepZoom(z, 1))}
          onZoomOut={() => setZoom((z) => stepZoom(z, -1))}
          onZoomChange={setZoom}
          onRotate={(direction) => setRotation((r) => rotate(r, direction))}
          onSearch={() => setSearchOpen((open) => !open)}
          onThemeChange={setTheme}
          onShowShortcuts={() => setDialog("shortcuts")}
          onShowAbout={() => setDialog("about")}
        />
        <div className="relative flex min-h-0 flex-1">
          {sidebarOpen && document_ && (
            <Sidebar outline={document_.outline} currentPage={currentPage} onJumpToPage={goToPage} />
          )}
          <div className="flex min-w-0 flex-1 flex-col">
            {document_ && findings.length > 0 && !bannerDismissed && (
              <SecurityBanner findings={findings} onDismiss={() => setBannerDismissed(true)} />
            )}
            <main
              ref={canvasRef}
              aria-label={strings.canvas.label}
              data-region="canvas"
              data-drop-active={dropActive || undefined}
              tabIndex={-1}
              className="relative min-h-0 flex-1 overflow-auto bg-muted outline-none data-drop-active:outline-2 data-drop-active:-outline-offset-4 data-drop-active:outline-primary data-drop-active:outline-dashed"
              onPointerDown={() => {
                // A floating sidebar closes when the user goes back to the page.
                if (sidebarOpen && isNarrowWindow()) setSidebarOpen(false);
              }}
            >
              {state.kind === "empty" && <EmptyState onOpen={onOpen} />}
              {state.kind === "loading" && (
                <LoadingState displayName={state.displayName} delayMs={loadingDelayMs} />
              )}
              {state.kind === "error" && (
                <ErrorState code={state.code} displayName={state.displayName} onOpen={onOpen} onRetry={onRetry} />
              )}
              {document_ && (
                <DocumentView
                  ref={viewRef}
                  pages={document_.pages}
                  zoom={zoom}
                  rotation={rotation}
                  scrollContainer={canvasRef}
                  doc={document_.doc}
                  renderer={renderer}
                  onCurrentPageChange={setCurrentPage}
                />
              )}
            </main>
            {document_ && searchOpen && (
              <div className="pointer-events-none absolute inset-x-0 top-0 *:pointer-events-auto">
                <SearchBar onClose={() => setSearchOpen(false)} />
              </div>
            )}
          </div>
        </div>
        <StatusBar
          document={document_ ? { displayName: document_.displayName, currentPage, pageCount, zoom } : null}
        />
      </div>
      <ShortcutsDialog open={dialog === "shortcuts"} onOpenChange={(open) => setDialog(open ? "shortcuts" : null)} />
      <AboutDialog
        open={dialog === "about"}
        onOpenChange={(open) => setDialog(open ? "about" : null)}
        version={version}
      />
    </TooltipProvider>
  );
}
