import { useEffect, useMemo, useRef, useState } from "react";

import { TooltipProvider } from "@/components/ui/tooltip";
import { createLinkSource, type LinksApi } from "@/features/links/source";
import { useSearch, type SearchApi } from "@/features/search/useSearch";
import { SecurityBanner } from "@/features/security-banner/SecurityBanner";
import { SecurityDetails } from "@/features/security-banner/SecurityDetails";
import { hasBannerContent } from "@/features/security-banner/summary";
import { AboutDialog, ShortcutsDialog } from "@/features/shell/dialogs";
import { rotate, stepZoom, type Rotation, type ShellState, type Zoom } from "@/features/shell/model";
import { SearchBar } from "@/features/shell/SearchBar";
import { Sidebar } from "@/features/shell/Sidebar";
import { EmptyState, ErrorState, LoadingState } from "@/features/shell/states";
import { StatusBar } from "@/features/shell/StatusBar";
import { Toolbar } from "@/features/shell/Toolbar";
import { useShortcuts } from "@/features/shortcuts/useShortcuts";
import { useTheme } from "@/features/theme/useTheme";
import { DocumentView, type DocumentViewHandle } from "@/features/viewer/DocumentView";
import type { PageRenderer } from "@/features/viewer/renderer";
import type { OutlineView } from "@/features/outline/tree";
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
  /** The open document's outline (loaded separately so loading it does not reset the view). */
  outline?: OutlineView;
  /** Searches the open document; without it (demo data, tests) the search bar finds nothing. */
  searchApi?: SearchApi;
  /** The pages' links; without it (demo data, tests) pages have none. */
  linksApi?: LinksApi;
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
  outline,
  searchApi,
  linksApi,
  version = "0.1.0",
  loadingDelayMs,
}: ReaderShellProps) {
  const [theme, setTheme] = useTheme();
  const [sidebarOpen, setSidebarOpen] = useState(() => !isNarrowWindow());
  const [searchOpen, setSearchOpen] = useState(false);
  const [bannerDismissed, setBannerDismissed] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  /** What the link under the pointer does (status bar). */
  const [linkHover, setLinkHover] = useState<string | null>(null);
  const [zoom, setZoom] = useState<Zoom>("fitWidth");
  /** What a fit mode currently shows, so zoom steps continue from there. */
  const [fitPercent, setFitPercent] = useState(100);
  const [rotation, setRotation] = useState<Rotation>(0);
  const [currentPage, setCurrentPage] = useState(1);
  const [dialog, setDialog] = useState<"shortcuts" | "about" | null>(null);
  const pageInputRef = useRef<HTMLInputElement>(null);
  const canvasRef = useRef<HTMLElement>(null);
  const viewRef = useRef<DocumentViewHandle>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const detailsButtonRef = useRef<HTMLButtonElement>(null);

  const document_ = state.kind === "open" ? state.document : null;
  const pageCount = document_?.pages.length ?? 0;
  const search = useSearch({ api: searchApi, doc: document_?.doc, active: searchOpen });
  const linkSource = useMemo(() => (linksApi ? createLinkSource(linksApi) : undefined), [linksApi]);

  // Per-document view state starts fresh for every newly opened document (nothing is remembered).
  const [viewedDocument, setViewedDocument] = useState(document_);
  if (viewedDocument !== document_) {
    setViewedDocument(document_);
    setZoom("fitWidth");
    setRotation(0);
    setCurrentPage(1);
    setBannerDismissed(false);
    setDetailsOpen(false);
    setSearchOpen(false);
    setLinkHover(null);
  }

  const goToPage = (page: number) => {
    const clamped = Math.min(Math.max(page, 1), pageCount);
    setCurrentPage(clamped);
    viewRef.current?.scrollToPage(clamped);
  };

  const whenOpen = (action: () => void) => () => {
    if (document_) action();
  };

  const closeSearch = () => {
    search.clear();
    setSearchOpen(false);
    // Focus would otherwise fall back to the page body, outside every region.
    canvasRef.current?.focus();
  };
  // F3 works whether or not the bar is open: it opens the bar and searches right away.
  const findAgain = (direction: 1 | -1) => {
    if (!document_) return;
    setSearchOpen(true);
    search.submit(direction);
  };

  // Every newly selected hit is scrolled to a third of the way down the view.
  const { hits, current } = search.state;
  const currentHit = searchOpen ? hits[current] : undefined;
  useEffect(() => {
    if (currentHit) viewRef.current?.revealHit(currentHit);
  }, [currentHit]);

  useShortcuts({
    open: onOpen,
    close: onClose,
    // Inline: focusing the field uses a ref, which must not be touched during render.
    search: () => {
      if (!document_) return;
      setSearchOpen(true);
      searchInputRef.current?.focus();
      searchInputRef.current?.select();
    },
    findNext: () => findAgain(1),
    findPrevious: () => findAgain(-1),
    zoomIn: whenOpen(() => setZoom((z) => stepZoom(z, 1, fitPercent))),
    zoomOut: whenOpen(() => setZoom((z) => stepZoom(z, -1, fitPercent))),
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
  const scanComplete = document_?.scanComplete ?? true;
  const bannerShown = document_ !== null && hasBannerContent(findings, scanComplete) && !bannerDismissed;
  const closeDetails = () => {
    setDetailsOpen(false);
    detailsButtonRef.current?.focus();
  };

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
          onZoomIn={() => setZoom((z) => stepZoom(z, 1, fitPercent))}
          onZoomOut={() => setZoom((z) => stepZoom(z, -1, fitPercent))}
          onZoomChange={setZoom}
          onRotate={(direction) => setRotation((r) => rotate(r, direction))}
          onSearch={() => (searchOpen ? closeSearch() : setSearchOpen(true))}
          onThemeChange={setTheme}
          onShowShortcuts={() => setDialog("shortcuts")}
          onShowAbout={() => setDialog("about")}
        />
        <div className="relative flex min-h-0 flex-1">
          {sidebarOpen && document_ && (
            <Sidebar
              key={document_.doc ?? document_.displayName}
              outline={outline ?? document_.outline ?? { status: "none" }}
              currentPage={currentPage}
              onJumpToPage={goToPage}
            />
          )}
          <div className="flex min-w-0 flex-1 flex-col">
            {bannerShown && (
              <SecurityBanner
                findings={findings}
                scanComplete={scanComplete}
                detailsOpen={detailsOpen}
                detailsButtonRef={detailsButtonRef}
                onToggleDetails={() => setDetailsOpen((open) => !open)}
                onDismiss={() => {
                  setBannerDismissed(true);
                  setDetailsOpen(false);
                  canvasRef.current?.focus();
                }}
              />
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
                  onEffectiveZoomChange={setFitPercent}
                  onZoomStep={(direction) => setZoom((z) => stepZoom(z, direction, fitPercent))}
                  highlights={searchOpen && hits.length > 0 ? { hits, current } : undefined}
                  links={linkSource}
                  onLinkHover={setLinkHover}
                  onLinkActivate={(link) => {
                    // Links inside the document jump right away. Web and blocked links get their
                    // dialogs in MVP-12b; until then nothing happens.
                    if (link.target.kind === "page") goToPage(link.target.pageIndex + 1);
                  }}
                />
              )}
            </main>
            {document_ && searchOpen && (
              <div className="pointer-events-none absolute inset-x-0 top-0 *:pointer-events-auto">
                <SearchBar search={search} pageCount={pageCount} inputRef={searchInputRef} onClose={closeSearch} />
              </div>
            )}
          </div>
          {bannerShown && detailsOpen && (
            <SecurityDetails findings={findings} scanComplete={scanComplete} onClose={closeDetails} />
          )}
        </div>
        <StatusBar
          document={document_ ? { displayName: document_.displayName, currentPage, pageCount, zoom } : null}
          hoverTarget={linkHover ?? undefined}
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
