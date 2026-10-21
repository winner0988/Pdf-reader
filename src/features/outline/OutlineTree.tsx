import { Ban, ChevronDown, ChevronRight, ExternalLink } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { DeleteBookmarkDialog } from "@/features/outline/DeleteBookmarkDialog";
import {
  canDemote,
  canMoveDown,
  canMoveUp,
  canPromote,
  childrenCount,
  currentItem,
  getMoveParams,
  hasChildren,
  initiallyExpanded,
  pageOf,
  parentOf,
  visibleAncestor,
  visibleItems,
  type MoveDirection,
} from "@/features/outline/tree";
import { strings } from "@/i18n/zh-TW";
import type { Edit, LinkTarget, OutlineItem } from "@/ipc/generated/contract";

const t = strings.sidebar;

export type OutlineEditing = {
  allowed: boolean;
  apply: (edit: Edit) => Promise<void>;
};

type OutlineTreeProps = {
  items: OutlineItem[];
  /** 1-based page being read; its item is marked. */
  currentPage: number;
  /** 1-based page to jump to. */
  onJumpToPage: (page: number) => void;
  /** An item pointing outside the document (a web link or a blocked action) was activated. */
  onOpenLink?: (item: number, target: Exclude<LinkTarget, { kind: "page" }>) => void;
  /** Editing operations (B3-17). */
  editing?: OutlineEditing;
};

/**
 * The outline as a keyboard tree (docs/ux/screen-map.md, section 7): ↑/↓ move, → expands or
 * enters, ← collapses or goes to the parent, Enter jumps. Titles are plain text (React escapes
 * them; the worker already removed control and bidi characters). Items pointing outside the
 * document are handed to `onOpenLink`, which confirms or explains (#49); nothing opens here.
 * Right-click menu and keyboard shortcuts provide bookmark editing (B3-17).
 */
export function OutlineTree({ items, currentPage, onJumpToPage, onOpenLink, editing }: OutlineTreeProps) {
  const [expanded, setExpanded] = useState(() => initiallyExpanded(items));
  const [focused, setFocused] = useState(0);
  const [renamingIndex, setRenamingIndex] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<OutlineItem | null>(null);
  const refs = useRef(new Map<number, HTMLLIElement>());
  const renameInputRef = useRef<HTMLInputElement>(null);

  const visible = visibleItems(items, expanded);
  const current = visibleAncestor(items, visible, currentItem(items, currentPage));
  // Focus lands on the item for the current page when the tree is entered.
  const tabStop = visible.includes(focused) ? focused : current >= 0 ? current : visible[0]!;

  useEffect(() => {
    if (renamingIndex !== null) {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    }
  }, [renamingIndex]);

  const moveTo = (index: number) => {
    setFocused(index);
    refs.current.get(index)?.focus();
  };

  const setOpen = (index: number, open: boolean) =>
    setExpanded((before) => {
      const after = new Set(before);
      if (open) after.add(index);
      else after.delete(index);
      return after;
    });

  const activate = (index: number) => {
    if (renamingIndex !== null) return;
    const target = items[index]!.target;
    const page = pageOf(items[index]!);
    if (page !== null) onJumpToPage(page + 1);
    else if (target && target.kind !== "page") onOpenLink?.(index, target);
  };

  const startRename = (index: number) => {
    if (!editing?.allowed) return;
    setRenamingIndex(index);
    setRenameValue(items[index]!.title);
  };

  const commitRename = (index: number) => {
    if (renamingIndex !== index) return;
    const clean = renameValue.trim();
    const item = items[index];
    if (item && clean && clean !== item.title && editing?.allowed) {
      void editing.apply({
        kind: "renameBookmark",
        id: item.id,
        title: clean,
      });
    }
    setRenamingIndex(null);
    refs.current.get(index)?.focus();
  };

  const cancelRename = (index?: number) => {
    const idx = index ?? renamingIndex;
    setRenamingIndex(null);
    if (idx !== null && idx !== undefined) {
      refs.current.get(idx)?.focus();
    }
  };

  const promptDelete = (index: number) => {
    if (!editing?.allowed) return;
    const item = items[index];
    if (!item) return;
    if (hasChildren(items, index)) {
      setDeleteTarget(item);
    } else {
      void editing.apply({ kind: "deleteBookmark", id: item.id });
    }
  };

  const handleAddBookmark = (index: number) => {
    if (!editing?.allowed) return;
    const item = items[index];
    if (!item) return;
    const childIdx = childrenCount(items, index);
    void editing.apply({
      kind: "addBookmark",
      parent: item.id,
      index: childIdx,
      title: t.defaultBookmarkTitle(currentPage),
      page: currentPage - 1,
    });
  };

  const handleMove = (index: number, direction: MoveDirection) => {
    if (!editing?.allowed) return;
    const params = getMoveParams(items, index, direction);
    if (params) {
      void editing.apply({
        kind: "moveBookmark",
        id: params.id,
        parent: params.parent,
        index: params.index,
      });
    }
  };

  const onKeyDown = (event: KeyboardEvent, index: number) => {
    if (renamingIndex !== null) return;

    // Bookmark editing shortcuts (B3-17)
    if (editing?.allowed) {
      if (event.key === "F2") {
        event.preventDefault();
        event.stopPropagation();
        startRename(index);
        return;
      }
      if (event.key === "Delete") {
        event.preventDefault();
        event.stopPropagation();
        promptDelete(index);
        return;
      }
      if ((event.ctrlKey || event.metaKey) && (event.key === "b" || event.key === "B")) {
        event.preventDefault();
        event.stopPropagation();
        handleAddBookmark(index);
        return;
      }
      if (event.altKey) {
        let direction: MoveDirection | null = null;
        if (event.key === "ArrowUp") direction = "up";
        else if (event.key === "ArrowDown") direction = "down";
        else if (event.key === "ArrowLeft") direction = "promote";
        else if (event.key === "ArrowRight") direction = "demote";

        if (direction) {
          event.preventDefault();
          event.stopPropagation();
          handleMove(index, direction);
          return;
        }
      }
    }

    const at = visible.indexOf(index);
    const parent = hasChildren(items, index);
    const open = expanded.has(index);
    let handled = true;
    switch (event.key) {
      case "ArrowDown":
        if (at + 1 < visible.length) moveTo(visible[at + 1]!);
        break;
      case "ArrowUp":
        if (at > 0) moveTo(visible[at - 1]!);
        break;
      case "ArrowRight":
        if (parent && !open) setOpen(index, true);
        else if (parent) moveTo(index + 1);
        break;
      case "ArrowLeft":
        if (parent && open) setOpen(index, false);
        else if (parentOf(items, index) >= 0) moveTo(parentOf(items, index));
        break;
      case "Home":
        moveTo(visible[0]!);
        break;
      case "End":
        moveTo(visible[visible.length - 1]!);
        break;
      case "Enter":
      case " ":
        activate(index);
        break;
      default:
        handled = false;
    }
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };

  const canEdit = editing?.allowed === true;

  return (
    <>
      <ul role="tree" aria-label={t.outlineTab} className="space-y-0.5">
        {visible.map((index) => {
          const item = items[index]!;
          const parent = hasChildren(items, index);
          const open = expanded.has(index);
          const target = item.target;
          const isRenaming = renamingIndex === index;

          return (
            <ContextMenu key={item.id ?? index}>
              <ContextMenuTrigger
                render={
                  <li
                    ref={(element) => {
                      if (element) refs.current.set(index, element);
                      else refs.current.delete(index);
                    }}
                    role="treeitem"
                    aria-level={item.depth + 1}
                    aria-expanded={parent ? open : undefined}
                    aria-current={index === current ? "location" : undefined}
                    tabIndex={index === tabStop ? 0 : -1}
                    onKeyDown={(event) => onKeyDown(event, index)}
                    onFocus={() => setFocused(index)}
                    onClick={() => {
                      setFocused(index);
                      activate(index);
                    }}
                    className="flex cursor-default items-center gap-1 rounded-md py-1 pr-2 text-sm outline-none hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring aria-[current=location]:bg-primary/10 aria-[current=location]:font-medium"
                    style={{ paddingLeft: `${0.25 + item.depth * 1}rem` }}
                  />
                }
              >
                {parent ? (
                  <button
                    type="button"
                    tabIndex={-1}
                    aria-label={open ? t.outlineCollapse : t.outlineExpand}
                    className="grid size-5 shrink-0 place-items-center rounded text-muted-foreground hover:bg-muted-foreground/10"
                    onClick={(event) => {
                      event.stopPropagation();
                      setOpen(index, !open);
                    }}
                  >
                    {open ? <ChevronDown className="size-4" aria-hidden /> : <ChevronRight className="size-4" aria-hidden />}
                  </button>
                ) : (
                  <span className="size-5 shrink-0" aria-hidden />
                )}
                {isRenaming ? (
                  <input
                    ref={renameInputRef}
                    type="text"
                    aria-label={t.renameInputLabel}
                    value={renameValue}
                    onChange={(e) => setRenameValue(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") {
                        e.stopPropagation();
                        e.preventDefault();
                        commitRename(index);
                      } else if (e.key === "Escape") {
                        e.stopPropagation();
                        e.preventDefault();
                        cancelRename(index);
                      }
                    }}
                    onBlur={() => commitRename(index)}
                    onClick={(e) => e.stopPropagation()}
                    className="h-6 min-w-0 flex-1 rounded border border-ring bg-background px-1.5 py-0.5 text-xs text-foreground outline-none focus:ring-1 focus:ring-ring"
                  />
                ) : (
                  <span className="min-w-0 flex-1 truncate" title={item.title}>
                    {item.title}
                  </span>
                )}
                {target?.kind === "uri" && (
                  <ExternalLink className="size-3.5 shrink-0 text-muted-foreground" role="img" aria-label={t.outlineExternalLink} />
                )}
                {target?.kind === "blocked" && (
                  <Ban className="size-3.5 shrink-0 text-muted-foreground" role="img" aria-label={t.outlineBlockedAction} />
                )}
              </ContextMenuTrigger>
              <ContextMenuContent>
                <ContextMenuItem disabled={!canEdit} onClick={() => handleAddBookmark(index)}>
                  {t.addBookmark}
                  <ContextMenuShortcut>Ctrl+B</ContextMenuShortcut>
                </ContextMenuItem>
                <ContextMenuItem disabled={!canEdit} onClick={() => startRename(index)}>
                  {t.renameBookmark}
                  <ContextMenuShortcut>F2</ContextMenuShortcut>
                </ContextMenuItem>
                <ContextMenuItem disabled={!canEdit} variant="destructive" onClick={() => promptDelete(index)}>
                  {t.deleteBookmark}
                  <ContextMenuShortcut>Delete</ContextMenuShortcut>
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem disabled={!canEdit || !canMoveUp(items, index)} onClick={() => handleMove(index, "up")}>
                  {t.moveUp}
                  <ContextMenuShortcut>Alt+↑</ContextMenuShortcut>
                </ContextMenuItem>
                <ContextMenuItem disabled={!canEdit || !canMoveDown(items, index)} onClick={() => handleMove(index, "down")}>
                  {t.moveDown}
                  <ContextMenuShortcut>Alt+↓</ContextMenuShortcut>
                </ContextMenuItem>
                <ContextMenuItem disabled={!canEdit || !canPromote(items, index)} onClick={() => handleMove(index, "promote")}>
                  {t.promote}
                  <ContextMenuShortcut>Alt+←</ContextMenuShortcut>
                </ContextMenuItem>
                <ContextMenuItem disabled={!canEdit || !canDemote(items, index)} onClick={() => handleMove(index, "demote")}>
                  {t.demote}
                  <ContextMenuShortcut>Alt+→</ContextMenuShortcut>
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          );
        })}
      </ul>
      <DeleteBookmarkDialog
        open={deleteTarget !== null}
        onConfirm={() => {
          if (deleteTarget && editing?.allowed) {
            void editing.apply({ kind: "deleteBookmark", id: deleteTarget.id });
          }
          setDeleteTarget(null);
        }}
        onCancel={() => setDeleteTarget(null)}
      />
    </>
  );
}
