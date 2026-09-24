import {
  ChevronLeft,
  FolderOpen,
  Minus,
  MoreHorizontal,
  PanelLeft,
  Plus,
  RotateCcw,
  RotateCw,
  Search,
} from "lucide-react";
import { useId, useState, type Ref } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
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
  onSetDefault: () => void;
  /** Prints the open document (MVP-17); without it the menu item is disabled. */
  onPrint?: () => void;
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
      {document && (
        <IconButton label={t.search} shortcut="Ctrl+F" pressed={props.searchOpen} onClick={props.onSearch}>
          <Search />
        </IconButton>
      )}
      <MoreMenu {...props} />
    </div>
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
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon" aria-label={t.more} />}>
        <MoreHorizontal />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48">
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
        <DropdownMenuItem disabled={!props.onPrint} onClick={props.onPrint}>
          {menu.print}
          <DropdownMenuShortcut>Ctrl+P</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem onClick={props.onShowShortcuts}>
          {menu.shortcuts}
          <DropdownMenuShortcut>Ctrl+/</DropdownMenuShortcut>
        </DropdownMenuItem>
        <DropdownMenuItem onClick={props.onShowAbout}>{menu.about}</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={props.onSetDefault}>{menu.setDefault}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
