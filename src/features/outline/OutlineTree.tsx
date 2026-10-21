import { Ban, ChevronDown, ChevronRight, ExternalLink, Plus } from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/button";
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
  addAfter,
  canDemote,
  canMoveDown,
  canMoveUp,
  canPromote,
  cleanTitle,
  currentItem,
  getMoveParams,
  hasChildren,
  initiallyExpanded,
  keyOf,
  pageOf,
  parentOf,
  visibleAncestor,
  visibleItems,
  type MoveDirection,
} from "@/features/outline/tree";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { Edit, LinkTarget, OutlineItem } from "@/ipc/generated/contract";

const t = strings.sidebar;

/** Bookmark editing in the outline (B3-17). */
export type OutlineEditing = {
  /** The document's author allows changing it. */
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
  /** Bookmark editing (B3-17). */
  editing?: OutlineEditing;
};

/** What to do when the outline after an edit arrives: until then, no other edit is made. */
type AfterEdit =
  /** Give the item (by its key) focus, its parents open. */
  | { kind: "focus"; key: number | null }
  /** Rename the item that was not there before: the one added. */
  | { kind: "added"; before: ReadonlySet<number> };

/**
 * The outline as a keyboard tree (docs/ux/screen-map.md, section 7): ↑/↓ move, → expands or
 * enters, ← collapses or goes to the parent, Enter jumps. Titles are plain text (React escapes
 * them; the worker already removed control and bidi characters). Items pointing outside the
 * document are handed to `onOpenLink`, which confirms or explains (#49); nothing opens here.
 *
 * With `editing`, bookmarks are added, renamed, deleted and moved from the context menu or the
 * keyboard (B3-17). Every edit gives the document a new outline; what is open and what has focus
 * follow the items by their bookmark ids ([`keyOf`]), so they stay with the item edited.
 */
export function OutlineTree({ items, currentPage, onJumpToPage, onOpenLink, editing }: OutlineTreeProps) {
  const keys = items.map(keyOf);
  const [expanded, setExpanded] = useState(
    () => new Set(Array.from(initiallyExpanded(items), (index) => keyOf(items[index]!, index))),
  );
  const [focused, setFocused] = useState<number | null>(null);
  const [renaming, setRenaming] = useState<{ key: number; title: string } | null>(null);
  const [deleting, setDeleting] = useState<{ item: OutlineItem; focusAfter: number | null } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [afterEdit, setAfterEdit] = useState<AfterEdit | null>(null);
  const [focusRequest, setFocusRequest] = useState<{ key: number } | null>(null);
  const [shownItems, setShownItems] = useState(items);
  const refs = useRef(new Map<number, HTMLLIElement>());
  const renameInput = useRef<HTMLInputElement>(null);
  const refocusAfterRename = useRef<number | null>(null);

  // The outline after an edit arrived: the item edited keeps focus, shown.
  if (shownItems !== items) {
    setShownItems(items);
    if (afterEdit) {
      setAfterEdit(null);
      const key = afterEdit.kind === "focus" ? afterEdit.key : keys.find((key) => !afterEdit.before.has(key));
      const index = key === undefined || key === null ? -1 : keys.indexOf(key);
      if (index >= 0) {
        setExpanded((before) => {
          const after = new Set(before);
          for (let parent = parentOf(items, index); parent >= 0; parent = parentOf(items, parent)) {
            after.add(keys[parent]!);
          }
          return after;
        });
        setFocused(keys[index]!);
        if (afterEdit.kind === "added") setRenaming({ key: keys[index]!, title: items[index]!.title });
        else setFocusRequest({ key: keys[index]! });
      }
    }
  }

  useEffect(() => {
    if (focusRequest) refs.current.get(focusRequest.key)?.focus();
  }, [focusRequest]);

  const renamingKey = renaming?.key ?? null;
  useEffect(() => {
    if (renamingKey !== null) {
      renameInput.current?.focus();
      renameInput.current?.select();
    } else if (refocusAfterRename.current !== null) {
      refs.current.get(refocusAfterRename.current)?.focus();
      refocusAfterRename.current = null;
    }
  }, [renamingKey]);

  const expandedIndices = new Set(keys.flatMap((key, index) => (expanded.has(key) ? [index] : [])));
  const visible = visibleItems(items, expandedIndices);
  const current = visibleAncestor(items, visible, currentItem(items, currentPage));
  const focusedIndex = focused === null ? -1 : keys.indexOf(focused);
  // Focus lands on the item for the current page when the tree is entered.
  const tabStop = visible.includes(focusedIndex) ? focusedIndex : current >= 0 ? current : visible[0]!;
  const canEdit = editing?.allowed === true && afterEdit === null;

  const moveTo = (index: number) => {
    setFocused(keys[index]!);
    refs.current.get(keys[index]!)?.focus();
  };

  const setOpen = (index: number, open: boolean) =>
    setExpanded((before) => {
      const after = new Set(before);
      if (open) after.add(keys[index]!);
      else after.delete(keys[index]!);
      return after;
    });

  const activate = (index: number) => {
    const target = items[index]!.target;
    const page = pageOf(items[index]!);
    if (page !== null) onJumpToPage(page + 1);
    else if (target && target.kind !== "page") onOpenLink?.(index, target);
  };

  /** Applies `change`; `after` says what has focus once the outline after it arrives. */
  const edit = (change: Edit, after: AfterEdit) => {
    if (!editing || !canEdit) return;
    setMessage(null);
    setAfterEdit(after);
    editing.apply(change).catch((error: unknown) => {
      setAfterEdit(null);
      // Too many unsaved changes to keep (B2-13): saving makes room.
      setMessage(errorCodeOf(error) === "limitExceeded" ? strings.pages.saveFirst : t.bookmarkFailed);
    });
  };

  /** Adds a bookmark for the page being read next to item `index` (-1: first at the top level). */
  const addBookmark = (index: number) => {
    const { parent, index: at } = addAfter(items, index);
    edit(
      { kind: "addBookmark", parent, index: at, title: t.defaultBookmarkTitle(currentPage), page: currentPage - 1 },
      { kind: "added", before: new Set(keys) },
    );
  };

  const startRename = (index: number) => {
    if (!canEdit) return;
    setRenaming({ key: keys[index]!, title: items[index]!.title });
  };

  /** Ends renaming, applying the title unless `apply` is false; `refocus` puts focus back on the item. */
  const endRename = (apply: boolean, refocus: boolean) => {
    if (!renaming) return;
    const index = keys.indexOf(renaming.key);
    const item = items[index];
    const title = cleanTitle(renaming.title);
    refocusAfterRename.current = refocus ? renaming.key : null;
    setRenaming(null);
    if (apply && item && title !== "" && title !== item.title) {
      edit({ kind: "renameBookmark", id: item.id, title }, { kind: "focus", key: renaming.key });
    }
  };

  /** The item that takes focus when item `index` and those under it are gone. */
  const neighbourAfterDelete = (index: number): number | null => {
    const depth = items[index]!.depth;
    let end = index + 1;
    while (end < items.length && items[end]!.depth > depth) end++;
    const next = visible.find((shown) => shown >= end);
    const before = visible.filter((shown) => shown < index).pop();
    const kept = next ?? before;
    return kept === undefined ? null : keys[kept]!;
  };

  const deleteBookmark = (index: number) => {
    if (!canEdit) return;
    const item = items[index]!;
    const focusAfter = neighbourAfterDelete(index);
    // Deleting the bookmarks under it too is asked first.
    if (hasChildren(items, index)) setDeleting({ item, focusAfter });
    else edit({ kind: "deleteBookmark", id: item.id }, { kind: "focus", key: focusAfter });
  };

  const move = (index: number, direction: MoveDirection) => {
    const params = getMoveParams(items, index, direction);
    if (params) edit({ kind: "moveBookmark", ...params }, { kind: "focus", key: keys[index]! });
  };

  const onKeyDown = (event: KeyboardEvent, index: number) => {
    if (renaming) return;
    if (editing?.allowed) {
      const ctrl = event.ctrlKey || event.metaKey;
      let editKey = true;
      if (event.key === "F2") startRename(index);
      else if (event.key === "Delete") deleteBookmark(index);
      else if (ctrl && !event.altKey && event.key.toLowerCase() === "b") {
        if (canEdit) addBookmark(index);
      } else if (event.altKey && !ctrl && event.key === "ArrowUp") move(index, "up");
      else if (event.altKey && !ctrl && event.key === "ArrowDown") move(index, "down");
      else if (event.altKey && !ctrl && event.key === "ArrowLeft") move(index, "promote");
      else if (event.altKey && !ctrl && event.key === "ArrowRight") move(index, "demote");
      else editKey = false;
      if (editKey) {
        event.preventDefault();
        event.stopPropagation();
        return;
      }
    }

    const at = visible.indexOf(index);
    const parent = hasChildren(items, index);
    const open = expandedIndices.has(index);
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

  return (
    <>
      {editing?.allowed && (
        <div className="mb-1 flex justify-end">
          <Button
            variant="ghost"
            size="sm"
            disabled={!canEdit}
            onClick={() => addBookmark(currentItem(items, currentPage))}
          >
            <Plus className="size-4" aria-hidden />
            {t.addBookmark}
          </Button>
        </div>
      )}
      {message && (
        <p role="alert" className="mb-1 rounded-md bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {message}
        </p>
      )}
      <ul role="tree" aria-label={t.outlineTab} aria-busy={afterEdit !== null} className="space-y-0.5">
        {visible.map((index) => {
          const item = items[index]!;
          const key = keys[index]!;
          const parent = hasChildren(items, index);
          const open = expandedIndices.has(index);
          const target = item.target;

          return (
            <ContextMenu key={key} disabled={!editing?.allowed}>
              <ContextMenuTrigger
                render={
                  <li
                    ref={(element) => {
                      if (element) refs.current.set(key, element);
                      else refs.current.delete(key);
                    }}
                    role="treeitem"
                    aria-level={item.depth + 1}
                    aria-expanded={parent ? open : undefined}
                    aria-current={index === current ? "location" : undefined}
                    tabIndex={index === tabStop ? 0 : -1}
                    onKeyDown={(event) => onKeyDown(event, index)}
                    onFocus={() => setFocused(key)}
                    onClick={() => {
                      setFocused(key);
                      if (renaming?.key !== key) activate(index);
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
                {renaming?.key === key ? (
                  <input
                    ref={renameInput}
                    type="text"
                    aria-label={t.renameInputLabel}
                    value={renaming.title}
                    onChange={(event) => setRenaming({ key, title: event.target.value })}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === "Escape") {
                        event.preventDefault();
                        endRename(event.key === "Enter", true);
                      }
                      // The tree's keys are the input's while renaming.
                      event.stopPropagation();
                    }}
                    onBlur={() => endRename(true, false)}
                    onClick={(event) => event.stopPropagation()}
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
              {editing?.allowed && (
                <ContextMenuContent>
                  <ContextMenuItem disabled={!canEdit} onClick={() => addBookmark(index)}>
                    {t.addBookmark}
                    <ContextMenuShortcut>Ctrl+B</ContextMenuShortcut>
                  </ContextMenuItem>
                  <ContextMenuItem disabled={!canEdit} onClick={() => startRename(index)}>
                    {t.renameBookmark}
                    <ContextMenuShortcut>F2</ContextMenuShortcut>
                  </ContextMenuItem>
                  <ContextMenuItem disabled={!canEdit} variant="destructive" onClick={() => deleteBookmark(index)}>
                    {t.deleteBookmark}
                    <ContextMenuShortcut>Delete</ContextMenuShortcut>
                  </ContextMenuItem>
                  <ContextMenuSeparator />
                  <ContextMenuItem disabled={!canEdit || !canMoveUp(items, index)} onClick={() => move(index, "up")}>
                    {t.moveUp}
                    <ContextMenuShortcut>Alt+↑</ContextMenuShortcut>
                  </ContextMenuItem>
                  <ContextMenuItem disabled={!canEdit || !canMoveDown(items, index)} onClick={() => move(index, "down")}>
                    {t.moveDown}
                    <ContextMenuShortcut>Alt+↓</ContextMenuShortcut>
                  </ContextMenuItem>
                  <ContextMenuItem disabled={!canEdit || !canPromote(items, index)} onClick={() => move(index, "promote")}>
                    {t.promote}
                    <ContextMenuShortcut>Alt+←</ContextMenuShortcut>
                  </ContextMenuItem>
                  <ContextMenuItem disabled={!canEdit || !canDemote(items, index)} onClick={() => move(index, "demote")}>
                    {t.demote}
                    <ContextMenuShortcut>Alt+→</ContextMenuShortcut>
                  </ContextMenuItem>
                </ContextMenuContent>
              )}
            </ContextMenu>
          );
        })}
      </ul>
      <DeleteBookmarkDialog
        open={deleting !== null}
        onConfirm={() => {
          if (deleting) edit({ kind: "deleteBookmark", id: deleting.item.id }, { kind: "focus", key: deleting.focusAfter });
          setDeleting(null);
        }}
        onCancel={() => setDeleting(null)}
      />
    </>
  );
}

/** An outline with no bookmark: a first one for the page being read (B3-17). */
export function AddFirstBookmark({ currentPage, editing }: { currentPage: number; editing: OutlineEditing }) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const add = () => {
    setBusy(true);
    setMessage(null);
    editing
      .apply({ kind: "addBookmark", parent: null, index: 0, title: t.defaultBookmarkTitle(currentPage), page: currentPage - 1 })
      .then(
        () => setBusy(false),
        (error: unknown) => {
          setBusy(false);
          setMessage(errorCodeOf(error) === "limitExceeded" ? strings.pages.saveFirst : t.bookmarkFailed);
        },
      );
  };
  return (
    <>
      <Button variant="outline" size="sm" disabled={busy || !editing.allowed} onClick={add}>
        <Plus className="size-4" aria-hidden />
        {t.addBookmark}
      </Button>
      {message && (
        <p role="alert" className="text-xs text-destructive">
          {message}
        </p>
      )}
    </>
  );
}
