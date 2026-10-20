// The row of tabs (MVP-14, docs/ux/screen-map.md). Each tab is a button with role "tab"; its close
// button sits outside role="tablist" (#188), so no interactive element is in the tablist.

import { LoaderCircle, LockKeyhole, Plus, TriangleAlert, X } from "lucide-react";
import { useRef, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/button";
import { IconButton } from "@/features/shell/IconButton";
import { isUnsaved, tabElementId, tabPanelId, type Tab } from "@/features/tabs/model";
import { strings } from "@/i18n/zh-TW";
import type { TabId } from "@/ipc/generated/contract";
import { cn } from "@/lib/utils";

const t = strings.tabs;

/** Shared sizing and geometry for each tab slot so visual and overlay layers stay aligned (#188). */
const tabSlotClass = "flex h-8 max-w-56 min-w-28 shrink-0 items-center rounded-t-md border border-b-0 text-sm";
const tabInnerClass = "flex h-full min-w-0 flex-1 items-center gap-1.5 pr-8 pl-3";

type TabLabelProps = {
  tab: Tab;
  /** Whether to render sr-only descriptions (loading, locked, failed, unsaved). Defaults to true. */
  includeSrOnly?: boolean;
};

/** Inner contents of a tab button (status icon, unsaved dot, truncated display name). */
function TabLabel({ tab, includeSrOnly = true }: TabLabelProps) {
  return (
    <>
      {tab.content.kind === "loading" && (
        <>
          <LoaderCircle className="size-3.5 shrink-0 animate-spin" aria-hidden />
          {includeSrOnly && <span className="sr-only">{t.loading}</span>}
        </>
      )}
      {tab.content.kind === "password" && (
        <>
          <LockKeyhole className="size-3.5 shrink-0" aria-hidden />
          {includeSrOnly && <span className="sr-only">{t.locked}</span>}
        </>
      )}
      {tab.content.kind === "error" && (
        <>
          <TriangleAlert className="size-3.5 shrink-0 text-destructive" aria-hidden />
          {includeSrOnly && <span className="sr-only">{t.failed}</span>}
        </>
      )}
      {isUnsaved(tab) && (
        <span aria-hidden className="shrink-0 font-bold">
          •
        </span>
      )}
      <span className="truncate">{tab.displayName}</span>
      {includeSrOnly && isUnsaved(tab) && <span className="sr-only">{t.unsaved}</span>}
    </>
  );
}

type TabBarProps = {
  tabs: Tab[];
  active: TabId | null;
  onActivate: (tab: TabId) => void;
  onClose: (tab: TabId) => void;
  onOpen: () => void;
};

export function TabBar({ tabs, active, onActivate, onClose, onOpen }: TabBarProps) {
  const buttons = useRef(new Map<TabId, HTMLButtonElement>());

  /** Arrow keys, Home and End move between tabs and show the one reached (automatic activation). Delete closes the tab. */
  function onKeyDown(event: KeyboardEvent, index: number) {
    if (event.key === "Delete") {
      event.preventDefault();
      const tab = tabs[index]?.tab;
      if (tab !== undefined) onClose(tab);
      return;
    }

    const last = tabs.length - 1;
    const target =
      event.key === "ArrowRight"
        ? (index + 1) % tabs.length
        : event.key === "ArrowLeft"
          ? (index - 1 + tabs.length) % tabs.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : null;
    const tab = target === null ? undefined : tabs[target]?.tab;
    if (tab === undefined) return;
    event.preventDefault();
    onActivate(tab);
    buttons.current.get(tab)?.focus();
  }

  return (
    <nav className="flex h-10 shrink-0 items-end gap-1 border-b bg-muted/60 px-2">
      <div className="flex min-w-0 flex-1 items-end overflow-x-auto">
        <div className="grid items-end">
          {/* Layer 1: The ARIA tablist containing only tabs */}
          <div role="tablist" aria-label={t.label} className="col-start-1 row-start-1 flex items-end gap-1">
            {tabs.map((tab, index) => {
              const selected = tab.tab === active;
              return (
                <div
                  key={tab.tab}
                  role="presentation"
                  data-selected={selected || undefined}
                  className={cn(
                    tabSlotClass,
                    selected ? "bg-background text-foreground" : "bg-muted text-muted-foreground hover:bg-background/70",
                  )}
                  onAuxClick={(event) => {
                    // Middle click closes a tab, as in a browser.
                    if (event.button === 1) {
                      event.preventDefault();
                      onClose(tab.tab);
                    }
                  }}
                >
                  <button
                    ref={(element) => {
                      if (element) buttons.current.set(tab.tab, element);
                      else buttons.current.delete(tab.tab);
                    }}
                    type="button"
                    role="tab"
                    id={tabElementId(tab.tab)}
                    aria-selected={selected}
                    aria-controls={tabPanelId(tab.tab)}
                    tabIndex={selected ? 0 : -1}
                    title={tab.displayName}
                    className={cn(
                      tabInnerClass,
                      "rounded-t-md outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    )}
                    onClick={() => onActivate(tab.tab)}
                    onKeyDown={(event) => onKeyDown(event, index)}
                  >
                    <TabLabel tab={tab} />
                  </button>
                </div>
              );
            })}
          </div>

          {/* Layer 2: Close buttons placed outside the tablist to satisfy ARIA required children (#188) */}
          <div className="pointer-events-none col-start-1 row-start-1 flex items-end gap-1">
            {tabs.map((tab) => {
              const selected = tab.tab === active;
              return (
                <div
                  key={tab.tab}
                  className={cn(tabSlotClass, "relative border-transparent")}
                >
                  {/* Invisible sizing counterpart matching tab content to guarantee identical width */}
                  <div aria-hidden="true" className={cn(tabInnerClass, "invisible")}>
                    <TabLabel tab={tab} includeSrOnly={false} />
                  </div>
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    className="pointer-events-auto absolute right-1 top-1 shrink-0"
                    aria-label={t.close(tab.displayName)}
                    tabIndex={selected ? 0 : -1}
                    onClick={() => onClose(tab.tab)}
                    onAuxClick={(event) => {
                      if (event.button === 1) {
                        event.preventDefault();
                        onClose(tab.tab);
                      }
                    }}
                  >
                    <X aria-hidden />
                  </Button>
                </div>
              );
            })}
          </div>
        </div>
      </div>
      <div className="mb-1 shrink-0">
        <IconButton label={t.open} shortcut="Ctrl+O" onClick={onOpen}>
          <Plus aria-hidden />
        </IconButton>
      </div>
    </nav>
  );
}
