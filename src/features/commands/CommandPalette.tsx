import { useEffect, useId, useRef, useState } from "react";
import { Search } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { strings } from "@/i18n/zh-TW";
import { cn } from "@/lib/utils";
import { filterCommands, recordRecentCommand } from "./registry";
import type { Command } from "./types";

export type CommandPaletteProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  commands: Command[];
};

function CommandPaletteContent({
  onOpenChange,
  commands,
}: {
  onOpenChange: (open: boolean) => void;
  commands: Command[];
}) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const selectedItemRef = useRef<HTMLLIElement>(null);
  const listboxId = useId();

  const filtered = filterCommands(commands, query);
  const activeIndex = selectedIndex < filtered.length ? selectedIndex : 0;

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    selectedItemRef.current?.scrollIntoView?.({ block: "nearest" });
  }, [activeIndex]);

  const handleExecute = (cmd: Command) => {
    if (!cmd.enabled) return;
    recordRecentCommand(cmd.id);
    onOpenChange(false);
    cmd.run();
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (filtered.length > 0) {
        setSelectedIndex((activeIndex + 1) % filtered.length);
      }
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (filtered.length > 0) {
        setSelectedIndex((activeIndex - 1 + filtered.length) % filtered.length);
      }
    } else if (e.key === "Home") {
      e.preventDefault();
      if (filtered.length > 0) {
        setSelectedIndex(0);
      }
    } else if (e.key === "End") {
      e.preventDefault();
      if (filtered.length > 0) {
        setSelectedIndex(filtered.length - 1);
      }
    } else if (e.key === "Enter") {
      e.preventDefault();
      const cmd = filtered[activeIndex];
      if (cmd) {
        handleExecute(cmd);
      }
    }
  };

  const activeCommand = filtered[activeIndex];
  const activeDescendantId = activeCommand ? `command-item-${activeCommand.id}` : undefined;

  return (
    <>
      <div className="flex items-center border-b px-3">
        <Search className="mr-2 h-4 w-4 shrink-0 opacity-50" />
        <input
          ref={inputRef}
          role="combobox"
          aria-label={strings.commands.palette}
          aria-expanded={filtered.length > 0}
          aria-haspopup="listbox"
          aria-controls={listboxId}
          aria-activedescendant={activeDescendantId}
          aria-autocomplete="list"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setSelectedIndex(0);
          }}
          onKeyDown={handleKeyDown}
          placeholder={strings.commands.palettePlaceholder}
          className="flex h-11 w-full rounded-md bg-transparent py-3 text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed disabled:opacity-50"
        />
      </div>

      <div className="max-h-[320px] overflow-y-auto p-1">
        {filtered.length === 0 ? (
          <div role="status" className="py-6 text-center text-sm text-muted-foreground">
            {strings.commands.noResults}
          </div>
        ) : (
          <ul
            id={listboxId}
            role="listbox"
            aria-label={strings.commands.palette}
            className="space-y-0.5"
          >
            {filtered.map((cmd, index) => {
              const isSelected = index === activeIndex;
              const itemId = `command-item-${cmd.id}`;
              const reasonId = `command-reason-${cmd.id}`;

              return (
                <li
                  key={cmd.id}
                  id={itemId}
                  ref={isSelected ? selectedItemRef : undefined}
                  role="option"
                  aria-selected={isSelected}
                  aria-disabled={!cmd.enabled}
                  aria-describedby={!cmd.enabled && cmd.disabledReason ? reasonId : undefined}
                  onClick={() => handleExecute(cmd)}
                  onMouseEnter={() => setSelectedIndex(index)}
                  className={cn(
                    "relative flex cursor-pointer select-none items-center justify-between rounded-md px-3 py-2 text-sm outline-none transition-colors",
                    isSelected && "bg-accent text-accent-foreground",
                    !cmd.enabled && "opacity-50 cursor-not-allowed",
                  )}
                >
                  <div className="flex flex-col min-w-0 pr-2">
                    <span className="font-medium truncate">{cmd.name}</span>
                    {!cmd.enabled && cmd.disabledReason && (
                      <span id={reasonId} className="text-xs text-muted-foreground truncate">
                        {cmd.disabledReason}
                      </span>
                    )}
                  </div>
                  {cmd.shortcut && (
                    <kbd className="pointer-events-none shrink-0 inline-flex h-5 select-none items-center gap-1 rounded border bg-muted px-1.5 font-mono text-[10px] font-medium text-muted-foreground">
                      {cmd.shortcut}
                    </kbd>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </>
  );
}

export function CommandPalette({ open, onOpenChange, commands }: CommandPaletteProps) {
  const previousActiveElement = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (open) {
      previousActiveElement.current = document.activeElement as HTMLElement | null;
    }
  }, [open]);

  const handleOpenChange = (nextOpen: boolean) => {
    onOpenChange(nextOpen);
    if (!nextOpen && previousActiveElement.current) {
      const el = previousActiveElement.current;
      setTimeout(() => el.focus(), 0);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent
        className="max-h-[80vh] sm:max-w-xl p-0 gap-0 overflow-hidden shadow-2xl"
        showCloseButton={false}
      >
        <DialogHeader className="sr-only">
          <DialogTitle>{strings.commands.palette}</DialogTitle>
          <DialogDescription>{strings.commands.palettePlaceholder}</DialogDescription>
        </DialogHeader>

        {open && (
          <CommandPaletteContent
            key="palette-content"
            onOpenChange={handleOpenChange}
            commands={commands}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}
