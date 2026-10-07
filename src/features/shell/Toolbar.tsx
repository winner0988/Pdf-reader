import {
  ChevronDown,
  ChevronLeft,
  FolderOpen,
  Minus,
  MoreHorizontal,
  PanelLeft,
  Highlighter,
  Pencil,
  Plus,
  RotateCcw,
  RotateCw,
  Search,
  Stamp,
} from "lucide-react";
import { useId, useState, type Ref } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { IconButton } from "@/features/shell/IconButton";
import { ZOOM_LEVELS, type Zoom } from "@/features/shell/model";
import { INK_COLORS, INK_SWATCH, INK_WIDTHS, STAMPS } from "@/features/annotations/tools";
import type { HighlightColor, InkColor, InkWidth, StampName } from "@/ipc/generated/contract";
import type { ThemePreference } from "@/features/theme/useTheme";
import { strings } from "@/i18n/zh-TW";

const t = strings.toolbar;

export type ToolbarProps = {
  document: { pageCount: number; currentPage: number; zoom: Zoom } | null;
  sidebarOpen: boolean;
  searchOpen: boolean;
  theme: ThemePreference;
  pageInputRef?: Ref<HTMLInputElement>;
  onToggleSidebar: () => void;
  onOpen: () => void;
  onGoToPage: (page: number) => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  onZoomChange: (zoom: Zoom) => void;
  onRotate: (direction: 1 | -1) => void;
  onSearch: () => void;
  onThemeChange: (theme: ThemePreference) => void;
  onShowShortcuts: () => void;
  onShowAbout: () => void;
  /** Opens the settings (B2-12); without it the menu has no settings item. */
  onShowSettings?: () => void;
  onSetDefault: () => void;
  /** Prints the open document (MVP-17); without it the menu item is disabled. */
  onPrint?: () => void;
  /** The document's author forbids printing (MVP-19): the menu item says so. */
  printBlocked?: boolean;
  /** Saves the open document's changes (B2-02); without it (nothing to save) the item is disabled. */
  onSave?: () => void;
  /** Saves the open document as another file (B2-02). */
  onSaveAs?: () => void;
  /** Undoes the last edit, or makes the last undone one again (B2-05); without them the items are disabled. */
  onUndo?: () => void;
  onRedo?: () => void;
  /** Exports the open document's text or pages (B2-04); without it the menu item is disabled. */
  onExport?: () => void;
  /** The document's author forbids copying, which covers exporting (MVP-19). */
  exportBlocked?: boolean;
  /** Writes a copy without metadata (B2-03); without it the menu item is disabled. */
  onPrivacyExport?: () => void;
  /** An encrypted document has no privacy export (B2-03): the menu item says so. */
  privacyExportBlocked?: boolean;
  /**
   * Marks the selected text with the highlighter in the color it used last (B2-07); without a
   * click handler (no text is selected, or the author does not allow annotating) the button is
   * disabled.
   */
  highlight?: { color: HighlightColor; onClick?: () => void };
  /**
   * The pen (B2-08): turned on or off with its button, in the color and thickness the menu next
   * to it says. Without a toggle (the author does not allow annotating) the button is disabled.
   */
  pen?: {
    active: boolean;
    color: InkColor;
    width: InkWidth;
    onToggle?: () => void;
    onColor: (color: InkColor) => void;
    onWidth: (width: InkWidth) => void;
  };
  /**
   * The stamps (B2-08): the menu has them; choosing one is to put it on a page. `active` is the
   * one chosen ("picture": one of the user's own). Without `onChoose` and `onPick` (the author
   * does not allow annotating) the menu is disabled.
   */
  stamp?: {
    active: StampName | "picture" | null;
    onChoose?: (stamp: StampName) => void;
    /** Asks for a picture of the user's own to make a stamp of. */
    onPick?: () => void;
  };
  /**
   * Turns the form of the open document into page content (B2-09); given only for a document
   * with a form. Without a click handler (the author does not allow filling in forms) the item
   * says so.
   */
  flatten?: { onClick?: () => void };
  /**
   * Recognises the text of the document's scanned pages now (B2-10); given only where the main
   * process can (not for demo data). `running` while it is doing so.
   */
  ocr?: { running: boolean; onStart: () => void };
  /** The "⋯" menu opened. */
  onMoreMenuOpen?: () => void;
  /**
   * Whether the open document's file may be on the recent files list (#73): the menu offers
   * "不記錄此檔案". `recorded` is null until known.
   */
  recording?: { recorded: boolean | null; onChange: (record: boolean) => void };
};

export function Toolbar(props: ToolbarProps) {
  const { document } = props;
  return (
    <div
      role="toolbar"
      aria-label={strings.appName}
      data-region="toolbar"
      className="flex h-12 shrink-0 items-center gap-1 border-b bg-background px-2"
    >
      <IconButton label={t.toggleSidebar} shortcut="F4" pressed={props.sidebarOpen} onClick={props.onToggleSidebar}>
        {props.sidebarOpen ? <ChevronLeft /> : <PanelLeft />}
      </IconButton>
      <IconButton label={t.open} shortcut="Ctrl+O" onClick={props.onOpen}>
        <FolderOpen />
      </IconButton>

      {document && (
        <>
          <PageInput
            key={document.currentPage}
            pageCount={document.pageCount}
            currentPage={document.currentPage}
            inputRef={props.pageInputRef}
            onGoToPage={props.onGoToPage}
          />
          <div className="mx-1 h-6 w-px bg-border" aria-hidden />
          <IconButton label={t.zoomOut} shortcut="Ctrl+-" onClick={props.onZoomOut}>
            <Minus />
          </IconButton>
          <select
            aria-label={t.zoomLevel}
            className="h-8 rounded-md border bg-background px-2 text-sm"
            value={String(document.zoom)}
            onChange={(event) => {
              const value = event.target.value;
              props.onZoomChange(value === "fitWidth" || value === "fitPage" ? value : Number(value));
            }}
          >
            <option value="fitWidth">{t.fitWidth}</option>
            <option value="fitPage">{t.fitPage}</option>
            {typeof document.zoom === "number" && !ZOOM_LEVELS.includes(document.zoom) && (
              <option value={document.zoom}>{t.zoomPercent(document.zoom)}</option>
            )}
            {ZOOM_LEVELS.map((level) => (
              <option key={level} value={level}>
                {t.zoomPercent(level)}
              </option>
            ))}
          </select>
          <IconButton label={t.zoomIn} shortcut="Ctrl+=" onClick={props.onZoomIn}>
            <Plus />
          </IconButton>
          <div className="mx-1 h-6 w-px bg-border" aria-hidden />
          <IconButton label={t.rotateCcw} shortcut="Ctrl+[" onClick={() => props.onRotate(-1)}>
            <RotateCcw />
          </IconButton>
          <IconButton label={t.rotateCw} shortcut="Ctrl+]" onClick={() => props.onRotate(1)}>
            <RotateCw />
          </IconButton>
        </>
      )}

      <div className="flex-1" />
      {document && props.highlight && (
        <IconButton
          label={
            props.highlight.onClick
              ? t.highlight(strings.annotations.colors[props.highlight.color])
              : t.highlightNeedsText
          }
          disabled={!props.highlight.onClick}
          onClick={props.highlight.onClick}
        >
          <Highlighter />
        </IconButton>
      )}
      {document && props.pen && <PenControls pen={props.pen} />}
      {document && props.stamp && <StampMenu stamp={props.stamp} />}
      {document && (
        <IconButton label={t.search} shortcut="Ctrl+F" pressed={props.searchOpen} onClick={props.onSearch}>
          <Search />
        </IconButton>
      )}
      <MoreMenu {...props} />
    </div>
  );
}

/** The pen's button, and next to it the menu of its color and thickness. */
function PenControls({ pen }: { pen: NonNullable<ToolbarProps["pen"]> }) {
  const labels = strings.annotations;
  return (
    <>
      <IconButton
        label={labels.pen}
        // The reason, when it is disabled, is in the tooltip.
        shortcut={pen.onToggle ? undefined : labels.notAllowed}
        pressed={pen.active}
        disabled={!pen.onToggle}
        onClick={pen.onToggle}
      >
        <Pencil />
      </IconButton>
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label={labels.penStyle} disabled={!pen.onToggle} />}>
          <ChevronDown />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-40">
          <DropdownMenuGroup>
            <DropdownMenuRadioGroup value={pen.color} onValueChange={(value) => pen.onColor(value as InkColor)}>
              {INK_COLORS.map((color) => (
                <DropdownMenuRadioItem key={color} value={color}>
                  <span
                    aria-hidden
                    className="mr-2 inline-block size-3 rounded-full border border-black/20"
                    style={{ backgroundColor: INK_SWATCH[color] }}
                  />
                  {labels.inkColors[color]}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <DropdownMenuRadioGroup value={pen.width} onValueChange={(value) => pen.onWidth(value as InkWidth)}>
              {INK_WIDTHS.map((width) => (
                <DropdownMenuRadioItem key={width} value={width}>
                  {labels.inkWidths[width]}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}

/** The stamps: the menu of them, the one chosen marked. */
function StampMenu({ stamp }: { stamp: NonNullable<ToolbarProps["stamp"]> }) {
  const labels = strings.annotations;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            aria-label={labels.stamp}
            title={stamp.onChoose ? undefined : labels.notAllowed}
            disabled={!stamp.onChoose}
            data-stamp-button=""
            data-active={stamp.active !== null || undefined}
            className="data-active:bg-accent"
          />
        }
      >
        <Stamp />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-40">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{labels.stamp}</DropdownMenuLabel>
          {/* Items, not a radio group: a choice closes the menu, and the pointer is the page's. */}
          {STAMPS.map((name) => (
            <DropdownMenuItem key={name} aria-current={stamp.active === name || undefined} onClick={() => stamp.onChoose?.(name)}>
              {labels.stamps[name]}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          aria-current={stamp.active === "picture" || undefined}
          disabled={!stamp.onPick}
          onClick={stamp.onPick}
        >
          {labels.pickPicture}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function PageInput({
  pageCount,
  currentPage,
  inputRef,
  onGoToPage,
}: {
  pageCount: number;
  currentPage: number;
  inputRef?: Ref<HTMLInputElement>;
  onGoToPage: (page: number) => void;
}) {
  // Remounted (via key) whenever the current page changes, so the draft starts fresh.
  const [draft, setDraft] = useState(String(currentPage));
  const [invalid, setInvalid] = useState(false);
  // Unique per tab (MVP-14): every tab has a toolbar.
  const errorId = useId();

  return (
    <div className="relative flex items-center gap-1 text-sm">
      <input
        ref={inputRef}
        aria-label={t.pageNumber}
        aria-invalid={invalid}
        aria-describedby={invalid ? errorId : undefined}
        inputMode="numeric"
        className="h-8 w-12 rounded-md border bg-background px-1 text-center aria-invalid:border-destructive"
        value={draft}
        onChange={(event) => {
          setDraft(event.target.value);
          setInvalid(false);
        }}
        onFocus={(event) => event.target.select()}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          const page = Number(draft);
          if (Number.isInteger(page) && page >= 1 && page <= pageCount) {
            onGoToPage(page);
          } else {
            setInvalid(true);
          }
        }}
      />
      <span className="text-muted-foreground">{t.pageCount(pageCount)}</span>
      {invalid && (
        <span
          id={errorId}
          role="alert"
          className="absolute top-9 left-0 z-10 rounded-md border bg-popover px-2 py-1 whitespace-nowrap text-destructive shadow"
        >
          {t.pageOutOfRange(pageCount)}
        </span>
      )}
    </div>
  );
}

function MoreMenu(props: ToolbarProps) {
  const menu = strings.menu;
  return (
    <DropdownMenu onOpenChange={(open) => open && props.onMoreMenuOpen?.()}>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon" aria-label={t.more} />}>
        <MoreHorizontal />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{menu.appearance}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={props.theme}
            onValueChange={(value) => props.onThemeChange(value as ThemePreference)}
          >
            <DropdownMenuRadioItem value="system">{menu.themeSystem}</DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="light">{menu.themeLight}</DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="dark">{menu.themeDark}</DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuItem disabled={!props.onUndo} onClick={props.onUndo}>
          {menu.undo}
          <DropdownMenuShortcut>Ctrl+Z</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem disabled={!props.onRedo} onClick={props.onRedo}>
          {menu.redo}
          <DropdownMenuShortcut>Ctrl+Y</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem disabled={!props.onSave} onClick={props.onSave}>
          {menu.save}
          <DropdownMenuShortcut>Ctrl+S</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem disabled={!props.onSaveAs} onClick={props.onSaveAs}>
          {menu.saveAs}
          <DropdownMenuShortcut>Ctrl+Shift+S</DropdownMenuShortcut>
        </DropdownMenuItem>
        {props.ocr && (
          <DropdownMenuItem disabled={props.ocr.running} onClick={props.ocr.onStart}>
            {props.ocr.running ? strings.ocr.menuRunning : strings.ocr.menu}
          </DropdownMenuItem>
        )}
        {props.flatten && (
          <DropdownMenuItem disabled={!props.flatten.onClick} onClick={props.flatten.onClick}>
            {strings.forms.flatten}
            {!props.flatten.onClick && (
              <DropdownMenuShortcut>{strings.permissions.notAllowed}</DropdownMenuShortcut>
            )}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem disabled={!props.onExport} onClick={props.onExport}>
          {menu.export}
          {props.exportBlocked && <DropdownMenuShortcut>{strings.permissions.notAllowed}</DropdownMenuShortcut>}
        </DropdownMenuItem>
        <DropdownMenuItem disabled={!props.onPrivacyExport} onClick={props.onPrivacyExport}>
          {menu.privacyExport}
          {props.privacyExportBlocked && (
            <DropdownMenuShortcut>{strings.privacyExport.encrypted}</DropdownMenuShortcut>
          )}
        </DropdownMenuItem>
        <DropdownMenuItem disabled={!props.onPrint} onClick={props.onPrint}>
          {menu.print}
          <DropdownMenuShortcut>{props.printBlocked ? strings.permissions.notAllowed : "Ctrl+P"}</DropdownMenuShortcut>
        </DropdownMenuItem>
        {props.recording && (
          <DropdownMenuCheckboxItem
            checked={props.recording.recorded === false}
            disabled={props.recording.recorded === null}
            onCheckedChange={(checked) => props.recording?.onChange(!checked)}
          >
            {menu.dontRecord}
          </DropdownMenuCheckboxItem>
        )}
        <DropdownMenuItem onClick={props.onShowShortcuts}>
          {menu.shortcuts}
          <DropdownMenuShortcut>Ctrl+/</DropdownMenuShortcut>
        </DropdownMenuItem>
        {props.onShowSettings && (
          <DropdownMenuItem onClick={props.onShowSettings}>{menu.settings}</DropdownMenuItem>
        )}
        <DropdownMenuItem onClick={props.onShowAbout}>{menu.about}</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={props.onSetDefault}>{menu.setDefault}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
