import { useEffect, useId, useMemo, useRef, useState } from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { TooltipProvider } from "@/components/ui/tooltip";
import { HIGHLIGHT_COLORS, SWATCH } from "@/features/annotations/model";
import { NoteDialog } from "@/features/annotations/NoteDialog";
import { createAnnotationSource, type AnnotationsApi } from "@/features/annotations/source";
import { useAnnotations } from "@/features/annotations/useAnnotations";
import { FieldEdits, until } from "@/features/forms/edits";
import { FlattenDialog } from "@/features/forms/FlattenDialog";
import { createFormSource, type FormsApi } from "@/features/forms/source";
import { BlockedLinkDialog, LinkConfirmDialog } from "@/features/links/LinkDialogs";
import type { ExportApi } from "@/features/export/api";
import { ExportDialog } from "@/features/export/ExportDialog";
import { PrivacyExportDialog } from "@/features/saving/PrivacyExportDialog";
import { createLinkSource, type LinksApi } from "@/features/links/source";
import { ALL_PERMISSIONS, restrictionSummary } from "@/features/permissions/permissions";
import type { RecentApi } from "@/features/recent/api";
import type { UpdatesApi } from "@/features/settings/updates";
import type { SavingApi } from "@/features/saving/api";
import { saveAsHelps } from "@/features/saving/messages";
import { SaveFailedDialog } from "@/features/saving/SaveFailedDialog";
import { useSearch, type SearchApi } from "@/features/search/useSearch";
import { SettingsDialog } from "@/features/settings/SettingsDialog";
import { useSettings } from "@/features/settings/useSettings";
import { RecoveryBanner } from "@/features/recovery/RecoveryBanner";
import { SecurityBanner } from "@/features/security-banner/SecurityBanner";
import { SecurityDetails } from "@/features/security-banner/SecurityDetails";
import { hasBannerContent } from "@/features/security-banner/summary";
import { AboutDialog, SetDefaultFailedDialog, ShortcutsDialog } from "@/features/shell/dialogs";
import { rotate, sameSession, stepZoom, type Rotation, type ShellState, type Zoom } from "@/features/shell/model";
import { PrintDialog } from "@/features/print/PrintDialog";
import { PrintPages } from "@/features/print/PrintPages";
import { freePrintPages, LOW_RES_PRINT_DPI, PRINT_DPI, renderForPrint, type PrintPage } from "@/features/print/render";
import { SearchBar } from "@/features/shell/SearchBar";
import { Sidebar } from "@/features/shell/Sidebar";
import { EmptyState, ErrorState, LoadingState, PasswordState } from "@/features/shell/states";
import { StatusBar } from "@/features/shell/StatusBar";
import { Toolbar } from "@/features/shell/Toolbar";
import { useShortcuts } from "@/features/shortcuts/useShortcuts";
import type { SystemApi } from "@/features/system/defaultApp";
import { createTextSource, type TextApi } from "@/features/text/source";
import type { EditingApi } from "@/features/thumbnails/api";
import type { PageEditing, SavePages } from "@/features/thumbnails/Thumbnails";
import { SourcePasswordDialog } from "@/features/thumbnails/SourcePasswordDialog";
import { UndoPasswordDialog } from "@/features/thumbnails/UndoPasswordDialog";
import { useTheme } from "@/features/theme/useTheme";
import { DocumentView, type DocumentViewHandle } from "@/features/viewer/DocumentView";
import { errorCodeOf, type PageRenderer } from "@/features/viewer/renderer";
import type { OutlineView } from "@/features/outline/tree";
import { strings } from "@/i18n/zh-TW";
import type {
  BlockedAction,
  DocumentId,
  Edit,
  LinkPreview,
  LinkTarget,
  PageLink,
  PagesSource,
  SaveResult,
} from "@/ipc/generated/contract";

/**
 * A link dialog that is open: the confirmation of a web link (with how to open it: by the
 * page link's or outline item's id, never by URI), or why a link is blocked.
 */
type LinkDialog =
  | { kind: "confirm"; key: string; preview: LinkPreview; open: () => Promise<void> }
  | { kind: "blocked"; action: BlockedAction; content: string | null };

type ReaderShellProps = {
  state: ShellState;
  onOpen: () => void;
  onClose?: () => void;
  onRetry?: () => void;
  /** The password the user typed for an encrypted document (MVP-16). */
  onUnlock?: (password: string) => void;
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
  /** The pages' text for selecting and copying; without it (demo data, tests) none can be selected. */
  textApi?: TextApi;
  /** The pages' annotations (B2-07); without it (demo data, tests) pages have none to show. */
  annotationsApi?: AnnotationsApi;
  /** The pages' form fields (B2-09); without it (demo data, tests) pages have none to show. */
  formsApi?: FormsApi;
  /** Opens Windows Settings for "set as default"; without it (demo data, tests) nothing happens. */
  systemApi?: SystemApi;
  /** The recently opened files (#73); without it (demo data, tests) the start screen lists none. */
  recentApi?: RecentApi;
  /** Exports pages as text or PNG files (B2-04); without it (demo data, tests) nothing can be. */
  exportApi?: ExportApi;
  /** Saves the document (B2-02); without it (demo data, tests) nothing can be. */
  savingApi?: SavingApi;
  /** Page management in the thumbnails (B2-05); without it (demo data, tests) pages stay as they are. */
  editingApi?: EditingApi;
  /** The update check in the settings (#64); without it (demo data, tests) it is not offered. */
  updatesApi?: UpdatesApi;
  /**
   * The document of the tab as the main process last said it, which can be a moment ahead of the
   * state shown: the next field of a form, saving and closing must not use the document this
   * page was still showing (B2-09). Without it (demo data, tests) the document shown is the latest.
   */
  latest?: () => { doc: DocumentId; unsaved: boolean } | undefined;
  /** The values of the form on their way to the document (B2-09); shared, so that closing a tab can wait for them. */
  fieldEdits?: FieldEdits;
  /** Whether this shell is the one shown (MVP-14): a hidden tab shell handles no keys. */
  active?: boolean;
  version?: string;
  /** Delay before the loading state appears; tests pass 0. */
  loadingDelayMs?: number;
};

/** Below this width the sidebar floats over the canvas (docs/ux/screen-map.md, section 1). */
const OVERLAY_SIDEBAR_BELOW_PX = 960;

const isNarrowWindow = () => window.innerWidth < OVERLAY_SIDEBAR_BELOW_PX;

/** How long the status bar shows a hint, such as that a page has no text to select. */
const HINT_MS = 4000;

/** Text the WebView itself has selected (in a dialog, say): Ctrl+C copies that, not the PDF's. */
const hasPageSelection = () => (window.getSelection()?.toString() ?? "") !== "";

/** Region order for F6 / Shift+F6 (docs/ux/screen-map.md, section 7). */
const REGION_ORDER = ["toolbar", "banner", "sidebar", "canvas"];

/** Moves focus to the next or previous region of `shell`: every tab has its own regions (MVP-14). */
function focusRegion(shell: HTMLElement | null, direction: 1 | -1) {
  if (!shell) return;
  const regions = REGION_ORDER.map((name) =>
    shell.querySelector<HTMLElement>(`[data-region="${name}"]`),
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
  onUnlock,
  dropActive = false,
  renderer,
  outline,
  searchApi,
  linksApi,
  textApi,
  annotationsApi,
  formsApi,
  systemApi,
  recentApi,
  exportApi,
  savingApi,
  editingApi,
  updatesApi,
  latest,
  fieldEdits,
  active = true,
  version = "0.1.0",
  loadingDelayMs,
}: ReaderShellProps) {
  const [theme, setTheme] = useTheme();
  const [sidebarOpen, setSidebarOpen] = useState(() => !isNarrowWindow());
  const [searchOpen, setSearchOpen] = useState(false);
  /**
   * What the banner said each time the user closed it: it comes back only with something it did
   * not say then, as when the pages of a file with other content come in (B2-06).
   */
  const [bannerDismissed, setBannerDismissed] = useState<string[]>([]);
  /** The user left the changes an earlier run left for later (B2-13): offered again next time. */
  const [recoveryDismissed, setRecoveryDismissed] = useState(false);
  const [recoveryBusy, setRecoveryBusy] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  /** What the link under the pointer does (status bar). */
  const [linkHover, setLinkHover] = useState<string | null>(null);
  const [linkDialog, setLinkDialog] = useState<LinkDialog | null>(null);
  /** Whether any of the document's text is selected (MVP-15). */
  const [textSelected, setTextSelected] = useState(false);
  /** A passing message in the status bar; `id` counts them, so the same one shows anew each time. */
  const [hint, setHint] = useState<{ text: string; id: number } | null>(null);
  const [zoom, setZoom] = useState<Zoom>("fitWidth");
  /** What a fit mode currently shows, so zoom steps continue from there. */
  const [fitPercent, setFitPercent] = useState(100);
  const [rotation, setRotation] = useState<Rotation>(0);
  const [currentPage, setCurrentPage] = useState(1);
  const [dialog, setDialog] = useState<
    "shortcuts" | "about" | "settings" | "setDefaultFailed" | "print" | "export" | "privacyExport" | null
  >(
    null,
  );
  /** Why saving failed (B2-02), while that is shown. */
  const [saveFailed, setSaveFailed] = useState<unknown>(null);
  /** Pages rendered for printing, while the system's print dialog is up (MVP-17). */
  const [printPages, setPrintPages] = useState<PrintPage[] | null>(null);
  /** Whether the document's file may be on the recent files list (#73); null until asked. */
  const [recorded, setRecorded] = useState<boolean | null>(null);
  const pageInputRef = useRef<HTMLInputElement>(null);
  const shellRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLElement>(null);
  const viewRef = useRef<DocumentViewHandle>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const detailsButtonRef = useRef<HTMLButtonElement>(null);
  const detailsId = useId();

  const document_ = state.kind === "open" ? state.document : null;
  const pageCount = document_?.pages.length ?? 0;
  const permissions = document_?.permissions ?? ALL_PERMISSIONS;
  const search = useSearch({ api: searchApi, doc: document_?.doc, active: searchOpen });
  const linkSource = useMemo(() => (linksApi ? createLinkSource(linksApi) : undefined), [linksApi]);
  const textSource = useMemo(() => (textApi ? createTextSource(textApi) : undefined), [textApi]);
  const annotationSource = useMemo(
    () => (annotationsApi ? createAnnotationSource(annotationsApi) : undefined),
    [annotationsApi],
  );
  const formSource = useMemo(() => (formsApi ? createFormSource(formsApi) : undefined), [formsApi]);

  // Per-document view state starts fresh for every newly opened document (nothing is remembered);
  // an edit or saving (B2-02) changes the document shown, not which one it is.
  const [viewedDocument, setViewedDocument] = useState(document_);
  if (viewedDocument !== document_) {
    setViewedDocument(document_);
  }
  if (viewedDocument !== document_ && !sameSession(viewedDocument, document_)) {
    setZoom("fitWidth");
    setRotation(0);
    setCurrentPage(1);
    setBannerDismissed([]);
    setRecoveryDismissed(false);
    setDetailsOpen(false);
    setSearchOpen(false);
    setLinkHover(null);
    setLinkDialog(null);
    setTextSelected(false);
    setHint(null);
    setRecorded(null);
    if (dialog === "print" || dialog === "export" || dialog === "privacyExport") setDialog(null);
  }

  useEffect(() => {
    if (hint === null) return;
    const timer = window.setTimeout(() => setHint(null), HINT_MS);
    return () => window.clearTimeout(timer);
  }, [hint]);

  const showHint = (text: string) => setHint((last) => ({ text, id: (last?.id ?? 0) + 1 }));

  /**
   * Copies the selected text (MVP-15), unless the author forbids it (MVP-19). False when none is
   * selected: the key does what it usually does.
   */
  const copySelection = () => {
    const view = viewRef.current;
    if (!view?.hasSelection()) return false;
    if (!permissions.copy) {
      showHint(strings.permissions.copyBlocked);
      return true;
    }
    void view.selectedText().then((text) => {
      if (text) navigator.clipboard?.writeText(text).catch(() => {});
    });
    return true;
  };

  const goToPage = (page: number) => {
    const clamped = Math.min(Math.max(page, 1), pageCount);
    setCurrentPage(clamped);
    viewRef.current?.scrollToPage(clamped);
  };

  // "不記錄此檔案" (#73): asked for whenever the menu opens, so it is never stale. Not offered when
  // the settings say not to record any file (B2-12).
  const recordingAllowed = useSettings()?.settings.recordRecentFiles ?? true;
  const recording =
    recentApi && recordingAllowed && document_?.doc !== undefined ? { api: recentApi, doc: document_.doc } : null;
  const askRecording = () => {
    recording?.api.isRecorded(recording.doc).then(setRecorded, () => setRecorded(null));
  };
  const setRecording = (record: boolean) => {
    recording?.api.setRecorded(recording.doc, record).then(() => setRecorded(record), askRecording);
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

  // Printing needs the worker's pages: not for demo data. Without a document Ctrl+P still does
  // nothing, rather than the WebView printing the app itself.
  const printable = document_?.doc !== undefined && renderer !== undefined;
  const print = () => {
    if (!printable) return;
    if (permissions.print) setDialog("print");
    else showHint(strings.permissions.printBlocked);
  };

  // Exporting needs the worker's pages and the main process (not demo data); the author's
  // permission to copy covers it (MVP-19).
  const exportable = document_?.doc !== undefined && exportApi !== undefined;
  /** Pages chosen in the thumbnails to be saved as a file of their own (B2-06). */
  const [exportPreset, setExportPreset] = useState<number[] | null>(null);
  const savePages: SavePages | undefined =
    exportable && document_?.encrypted !== true
      ? {
          allowed: permissions.copy,
          open: (pages) => {
            setExportPreset(pages);
            setDialog("export");
          },
        }
      : undefined;

  // Saving (B2-02) needs the main process, which has the file: not for demo data.
  const doc = document_?.doc;
  const savable = doc !== undefined && savingApi !== undefined;
  const saved = (result: SaveResult | null) => {
    if (result) showHint(result.incremental ? strings.saving.savedIncremental : strings.saving.saved);
  };
  // A value still being typed in a field of the form is part of what is saved (B2-09), and the
  // edit that sent it gave the document a new id: saving looks at the document as it is then.
  const [ownEdits] = useState(() => new FieldEdits());
  const edits = fieldEdits ?? ownEdits;
  const latestDocument = () => latest?.() ?? (doc !== undefined ? { doc, unsaved: document_?.unsaved === true } : undefined);
  const save = () => {
    if (!savable) return;
    edits.whenSettled(
      () => {
        const now = latestDocument();
        if (now?.unsaved) savingApi.save(now.doc).then(saved, setSaveFailed);
      },
      { keepFocus: true },
    );
  };
  const saveAs = () => {
    setSaveFailed(null);
    if (!savable) return;
    edits.whenSettled(
      () => {
        const now = latestDocument();
        if (now) savingApi.saveAs(now.doc).then(saved, setSaveFailed);
      },
      { keepFocus: true },
    );
  };

  // Page management (B2-05) needs the main process too. The author's permission to assemble or
  // change the document covers it (MVP-19).
  // The pages of another file (B2-06): asked for in the main process's dialog; a file that needs
  // a password gets it asked here, one try after another until it opens or the user gives up.
  const [sourcePassword, setSourcePassword] = useState<{
    wrong: boolean;
    answer: (password: string | null) => void;
  } | null>(null);
  const insertFrom = async (at: number): Promise<number | null> => {
    const api = editingApi;
    if (!api?.pickPagesSource || doc === undefined) return null;
    let source: PagesSource | null;
    try {
      source = await api.pickPagesSource(doc);
    } catch (error) {
      if (errorCodeOf(error) !== "encrypted" || !api.unlockPagesSource) throw error;
      source = null;
      for (let wrong = false; ; wrong = true) {
        const password = await new Promise<string | null>((answer) => setSourcePassword({ wrong, answer }));
        if (password === null) break;
        try {
          source = await api.unlockPagesSource(doc, password);
          break;
        } catch (failure) {
          if (errorCodeOf(failure) !== "encrypted") {
            setSourcePassword(null);
            throw failure;
          }
        }
      }
      setSourcePassword(null);
    }
    if (!source) return null;
    await api.applyEdit(doc, { kind: "insertPages", at, source: source.source });
    return source.pages;
  };
  const pageEditing: PageEditing | undefined =
    doc !== undefined && editingApi !== undefined
      ? {
          allowed: permissions.assemble || permissions.modify,
          apply: (edit) => editingApi.applyEdit(doc, edit),
          insertFrom: editingApi.pickPagesSource ? insertFrom : undefined,
        }
      : undefined;
  // Undo and redo (B2-05). A document opened with a password needs it again to undo (#94): the
  // first try, without one, fails with `encrypted`, and the password is asked for.
  const undoable = pageEditing !== undefined && document_?.canUndo === true;
  const redoable = pageEditing !== undefined && document_?.canRedo === true;
  const [undoPassword, setUndoPassword] = useState<{ wrong: boolean } | null>(null);
  const undo = (password?: string) => {
    if (!undoable) return;
    editingApi!.undo(doc!, password).then(
      () => setUndoPassword(null),
      (error: unknown) => {
        if (errorCodeOf(error) === "encrypted") {
          setUndoPassword({ wrong: password !== undefined });
        } else {
          setUndoPassword(null);
          showHint(strings.pages.failed);
        }
      },
    );
  };
  const redo = () => {
    if (redoable) editingApi!.redo(doc!).catch(() => showHint(strings.pages.failed));
  };
  // Filling in the form (B2-09); the author's permission to fill in forms covers it, and flattening (MVP-19).
  const fillForms = permissions.fillForms && doc !== undefined && editingApi !== undefined;
  // One value after another: each edit gives the document a new id, which the next one must use.
  const editField = (edit: Edit): Promise<void> => {
    if (!fillForms) return Promise.reject(new Error("the fields cannot be changed"));
    return edits
      .run(async () => {
        const before = latestDocument();
        if (!before) throw new Error("no document");
        await editingApi!.applyEdit(before.doc, edit);
        // The document the edit made is announced on the open-events channel, which a big
        // document's announcement reaches a moment after the answer: the next edit and saving
        // need it.
        if (latest) await until(() => latest()?.doc !== before.doc);
      })
      .catch((error: unknown) => {
        showHint(errorCodeOf(error) === "limitExceeded" ? strings.pages.saveFirst : strings.forms.failed);
        throw error;
      });
  };
  const [flattenOpen, setFlattenOpen] = useState(false);
  // Flattened, the document is to be saved as another file: where is asked next (by the system).
  const flatten = () => {
    setFlattenOpen(false);
    editField({ kind: "flattenForm" }).then(
      () => saveAs(),
      () => showHint(strings.forms.flattenFailed),
    );
  };
  // Highlighter marks and notes (B2-07); the author's permission to annotate covers them (MVP-19).
  const annotations = useAnnotations({
    doc,
    editing: editingApi,
    allowed: permissions.annotate,
    view: viewRef,
    onProblem: showHint,
  });
  // Changes an earlier run of the app left for the file (B2-13): made again or discarded. The tab's
  // new state comes on the open-events channel.
  const recovery = document_?.recovery ?? "none";
  const recoveryShown = !recoveryDismissed && doc !== undefined && editingApi !== undefined;
  const answerRecovery = (restore: boolean) => {
    if (doc === undefined || editingApi === undefined) return;
    setRecoveryBusy(true);
    (restore ? editingApi.recover(doc) : editingApi.discardRecovered(doc))
      .catch((error: unknown) =>
        // Restoring is refused while the document has edits of its own.
        showHint(restore && errorCodeOf(error) === "invalidArgument" ? strings.recovery.ownChanges : strings.recovery.failed),
      )
      .finally(() => setRecoveryBusy(false));
  };
  // Deleting pages can leave the page being read past the end.
  if (pageCount > 0 && currentPage > pageCount) setCurrentPage(pageCount);

  useShortcuts({
    open: onOpen,
    // Always taken, even with nothing to save: the WebView would otherwise save the page.
    save,
    saveAs,
    undo: whenOpen(() => undo()),
    redo: whenOpen(redo),
    close: onClose,
    print,
    copy: () => !hasPageSelection() && copySelection(),
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
    nextRegion: () => focusRegion(shellRef.current, 1),
    previousRegion: () => focusRegion(shellRef.current, -1),
    help: () => setDialog("shortcuts"),
  }, active);

  /**
   * Links inside the document jump right away. A web link is described by the main process
   * first (from its id) and opened only after the user confirms; a blocked one only says why.
   */
  const activateLink = (link: PageLink) => {
    const target = link.target;
    if (target.kind === "page") {
      goToPage(target.pageIndex + 1);
    } else if (target.kind === "blocked") {
      setLinkDialog({ kind: "blocked", action: target.action, content: target.target });
    } else if (linksApi && document_?.doc !== undefined) {
      const doc = document_.doc;
      linksApi.describeLink(doc, link.id).then(
        (preview) =>
          setLinkDialog({
            kind: "confirm",
            key: `page:${link.id.pageIndex}:${link.id.index}`,
            preview,
            open: () => linksApi.openLink(doc, link.id),
          }),
        // The document closed or the worker failed: there is nothing to confirm.
        () => {},
      );
    }
  };

  /** The same for outline items that point outside the document (#49). */
  const openOutlineLink = (item: number, target: Exclude<LinkTarget, { kind: "page" }>) => {
    if (target.kind === "blocked") {
      setLinkDialog({ kind: "blocked", action: target.action, content: target.target });
    } else if (linksApi && document_?.doc !== undefined) {
      const doc = document_.doc;
      linksApi.describeOutlineLink(doc, item).then(
        (preview) =>
          setLinkDialog({
            kind: "confirm",
            key: `outline:${item}`,
            preview,
            open: () => linksApi.openOutlineLink(doc, item),
          }),
        () => {},
      );
    }
  };

  const findings = document_?.findings ?? [];
  const scanComplete = document_?.scanComplete ?? true;
  const bannerSays = JSON.stringify([findings, scanComplete]);
  const bannerShown =
    document_ !== null && hasBannerContent(findings, scanComplete) && !bannerDismissed.includes(bannerSays);
  const closeDetails = () => {
    setDetailsOpen(false);
    detailsButtonRef.current?.focus();
  };

  return (
    <TooltipProvider>
      <div ref={shellRef} className="flex h-full flex-col bg-background text-foreground">
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
          onShowSettings={() => setDialog("settings")}
          onSetDefault={() => {
            systemApi?.openDefaultAppsSettings().catch(() => setDialog("setDefaultFailed"));
          }}
          onPrint={printable && permissions.print ? print : undefined}
          printBlocked={!permissions.print}
          onSave={savable && document_?.unsaved ? save : undefined}
          onSaveAs={savable ? saveAs : undefined}
          onUndo={undoable ? () => undo() : undefined}
          onRedo={redoable ? redo : undefined}
          onExport={
            exportable && permissions.copy
              ? () => {
                  setExportPreset(null);
                  setDialog("export");
                }
              : undefined
          }
          exportBlocked={exportable && !permissions.copy}
          onPrivacyExport={savable && !document_?.encrypted ? () => setDialog("privacyExport") : undefined}
          privacyExportBlocked={savable && document_?.encrypted === true}
          flatten={
            document_?.hasForm && editingApi !== undefined
              ? { onClick: fillForms ? () => setFlattenOpen(true) : undefined }
              : undefined
          }
          highlight={
            editingApi !== undefined && doc !== undefined
              ? {
                  color: annotations.color,
                  onClick: annotations.enabled && textSelected ? () => annotations.highlight() : undefined,
                }
              : undefined
          }
          onMoreMenuOpen={askRecording}
          recording={recording ? { recorded, onChange: setRecording } : undefined}
        />
        <div className="relative flex min-h-0 flex-1">
          {sidebarOpen && document_ && (
            <Sidebar
              // One per opened file: an edit (B2-05) keeps the tab shown and the pages selected.
              key={document_.session ?? document_.doc ?? document_.displayName}
              outline={outline ?? document_.outline ?? { status: "none" }}
              pages={document_.pages}
              doc={document_.doc}
              renderer={renderer}
              currentPage={currentPage}
              onJumpToPage={goToPage}
              onOpenLink={openOutlineLink}
              pageEditing={pageEditing}
              savePages={savePages}
            />
          )}
          <div className="flex min-w-0 flex-1 flex-col">
            {recoveryShown && recovery !== "none" && (
              <RecoveryBanner
                recovery={recovery}
                busy={recoveryBusy}
                onRestore={() => answerRecovery(true)}
                onDiscard={() => answerRecovery(false)}
                onLater={() => {
                  setRecoveryDismissed(true);
                  canvasRef.current?.focus();
                }}
              />
            )}
            {bannerShown && (
              <SecurityBanner
                findings={findings}
                scanComplete={scanComplete}
                detailsOpen={detailsOpen}
                detailsId={detailsId}
                detailsButtonRef={detailsButtonRef}
                onToggleDetails={() => setDetailsOpen((open) => !open)}
                onDismiss={() => {
                  setBannerDismissed((said) => [...said, bannerSays]);
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
              // The scroll bar's space is always kept: otherwise a fitted page just taller than the
              // canvas makes the scroll bar appear, which narrows the canvas, which shrinks the page,
              // which hides the scroll bar again, forever (#46).
              className="relative min-h-0 flex-1 overflow-auto bg-muted outline-none [scrollbar-gutter:stable] data-drop-active:outline-2 data-drop-active:-outline-offset-4 data-drop-active:outline-primary data-drop-active:outline-dashed"
              onPointerDown={() => {
                // A floating sidebar closes when the user goes back to the page.
                if (sidebarOpen && isNarrowWindow()) setSidebarOpen(false);
              }}
            >
              {state.kind === "empty" && <EmptyState onOpen={onOpen} recent={recentApi} />}
              {state.kind === "loading" && (
                <LoadingState displayName={state.displayName} delayMs={loadingDelayMs} />
              )}
              {state.kind === "password" && (
                <PasswordState
                  displayName={state.displayName}
                  wrong={state.wrong}
                  onUnlock={(password) => onUnlock?.(password)}
                  onCancel={() => onClose?.()}
                />
              )}
              {state.kind === "error" && (
                <ErrorState code={state.code} displayName={state.displayName} onOpen={onOpen} onRetry={onRetry} />
              )}
              {document_ && (
                // Right-clicking the document shows the app's own menu, not the WebView's.
                <ContextMenu>
                  <ContextMenuTrigger
                    className="min-h-full"
                    onContextMenu={(event) => annotations.contextMenuAt(event.clientX, event.clientY)}
                  >
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
                      onLinkActivate={(link) => activateLink(link)}
                      text={textSource}
                      onSelectionChange={setTextSelected}
                      onNoText={() => showHint(strings.text.noTextLayer)}
                      annotations={annotationSource}
                      onAnnotationEdit={annotations.edit}
                      onEditNote={annotations.editNote}
                      forms={formSource}
                      onFieldEdit={fillForms ? editField : undefined}
                      onFieldScript={() => showHint(strings.forms.scriptNotRun)}
                    />
                  </ContextMenuTrigger>
                  <ContextMenuContent>
                    <ContextMenuItem disabled={!textSelected || !permissions.copy} onClick={() => copySelection()}>
                      {strings.text.copy}
                      <ContextMenuShortcut>{permissions.copy ? "Ctrl+C" : strings.permissions.notAllowed}</ContextMenuShortcut>
                    </ContextMenuItem>
                    {editingApi !== undefined && (
                      <>
                        <ContextMenuSeparator />
                        <ContextMenuSub>
                          <ContextMenuSubTrigger disabled={!textSelected || !annotations.enabled}>
                            {strings.annotations.highlight}
                          </ContextMenuSubTrigger>
                          <ContextMenuSubContent>
                            {HIGHLIGHT_COLORS.map((color) => (
                              <ContextMenuItem key={color} onClick={() => annotations.highlight(color)}>
                                <span
                                  aria-hidden
                                  className="size-3 rounded-full border border-black/20"
                                  style={{ backgroundColor: SWATCH[color] }}
                                />
                                {strings.annotations.colors[color]}
                              </ContextMenuItem>
                            ))}
                          </ContextMenuSubContent>
                        </ContextMenuSub>
                        <ContextMenuItem disabled={!annotations.enabled} onClick={() => annotations.newNote()}>
                          {strings.annotations.addNote}
                          {!permissions.annotate && (
                            <ContextMenuShortcut>{strings.permissions.notAllowed}</ContextMenuShortcut>
                          )}
                        </ContextMenuItem>
                      </>
                    )}
                  </ContextMenuContent>
                </ContextMenu>
              )}
            </main>
            {document_ && searchOpen && (
              <div className="pointer-events-none absolute inset-x-0 top-0 *:pointer-events-auto">
                <SearchBar search={search} pageCount={pageCount} inputRef={searchInputRef} onClose={closeSearch} />
              </div>
            )}
          </div>
          {bannerShown && detailsOpen && (
            <SecurityDetails id={detailsId} findings={findings} scanComplete={scanComplete} onClose={closeDetails} />
          )}
        </div>
        <StatusBar
          document={document_ ? { displayName: document_.displayName, currentPage, pageCount, zoom } : null}
          hoverTarget={linkHover ?? undefined}
          hint={hint?.text}
          restriction={document_ ? restrictionSummary(permissions) : null}
        />
      </div>
      {linkDialog?.kind === "confirm" && (
        <LinkConfirmDialog
          key={linkDialog.key}
          preview={linkDialog.preview}
          onOpen={linkDialog.open}
          onClose={() => setLinkDialog(null)}
        />
      )}
      {linkDialog?.kind === "blocked" && (
        <BlockedLinkDialog
          action={linkDialog.action}
          content={linkDialog.content}
          onClose={() => setLinkDialog(null)}
        />
      )}
      {flattenOpen && <FlattenDialog onConfirm={flatten} onCancel={() => setFlattenOpen(false)} />}
      {annotations.note && (
        <NoteDialog
          initial={annotations.note.kind === "edit" ? (annotations.note.annotation.text ?? "") : null}
          onSave={annotations.saveNote}
          onCancel={annotations.closeNote}
        />
      )}
      <SourcePasswordDialog
        open={sourcePassword !== null}
        wrong={sourcePassword?.wrong ?? false}
        onSubmit={(password) => sourcePassword?.answer(password)}
        onCancel={() => sourcePassword?.answer(null)}
      />
      <UndoPasswordDialog
        open={undoPassword !== null}
        wrong={undoPassword?.wrong ?? false}
        onUndo={undo}
        onCancel={() => setUndoPassword(null)}
      />
      <SaveFailedDialog
        failure={saveFailed}
        onSaveAs={savable && saveAsHelps(saveFailed) ? saveAs : undefined}
        onClose={() => setSaveFailed(null)}
      />
      <ShortcutsDialog open={dialog === "shortcuts"} onOpenChange={(open) => setDialog(open ? "shortcuts" : null)} />
      <SetDefaultFailedDialog
        open={dialog === "setDefaultFailed"}
        onOpenChange={(open) => setDialog(open ? "setDefaultFailed" : null)}
      />
      {printable && (
        <PrintDialog
          open={dialog === "print"}
          onOpenChange={(open) => setDialog(open ? "print" : null)}
          pageCount={pageCount}
          currentPage={currentPage}
          prepare={(pages, onProgress, signal) =>
            renderForPrint({
              renderer,
              doc: document_.doc!,
              pages,
              sizes: document_.pages,
              onProgress,
              signal,
              dpi: permissions.printHighQuality ? PRINT_DPI : LOW_RES_PRINT_DPI,
            })
          }
          onReady={setPrintPages}
          lowResolutionDpi={permissions.printHighQuality ? undefined : LOW_RES_PRINT_DPI}
        />
      )}
      {savable && (
        <PrivacyExportDialog
          open={dialog === "privacyExport"}
          onOpenChange={(open) => setDialog(open ? "privacyExport" : null)}
          onExport={() => savingApi.privacyExport(doc)}
          onFinished={showHint}
        />
      )}
      {exportable && (
        <ExportDialog
          open={dialog === "export"}
          onOpenChange={(open) => setDialog(open ? "export" : null)}
          doc={document_.doc!}
          pageCount={pageCount}
          currentPage={currentPage}
          encrypted={document_.encrypted === true}
          preset={exportPreset}
          api={exportApi}
          onFinished={showHint}
        />
      )}
      {printPages && (
        <PrintPages
          pages={printPages}
          onDone={() => {
            freePrintPages(printPages);
            setPrintPages(null);
          }}
        />
      )}
      <SettingsDialog
        open={dialog === "settings"}
        onOpenChange={(open) => setDialog(open ? "settings" : null)}
        recentApi={recentApi}
        updatesApi={updatesApi}
      />
      <AboutDialog
        open={dialog === "about"}
        onOpenChange={(open) => setDialog(open ? "about" : null)}
        version={version}
      />
    </TooltipProvider>
  );
}
