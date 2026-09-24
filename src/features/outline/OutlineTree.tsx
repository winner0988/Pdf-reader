import { Ban, ChevronDown, ChevronRight, ExternalLink } from "lucide-react";
import { useRef, useState, type KeyboardEvent } from "react";

import {
  currentItem,
  hasChildren,
  initiallyExpanded,
  pageOf,
  parentOf,
  visibleAncestor,
  visibleItems,
} from "@/features/outline/tree";
import { strings } from "@/i18n/zh-TW";
import type { OutlineItem } from "@/ipc/generated/contract";

const t = strings.sidebar;

type OutlineTreeProps = {
  items: OutlineItem[];
  /** 1-based page being read; its item is marked. */
  currentPage: number;
  /** 1-based page to jump to. */
  onJumpToPage: (page: number) => void;
};

/**
 * The outline as a keyboard tree (docs/ux/screen-map.md, section 7): ↑/↓ move, → expands or
 * enters, ← collapses or goes to the parent, Enter jumps. Titles are plain text (React escapes
 * them; the worker already removed control and bidi characters). Items pointing outside the
 * document never do anything here; MVP-12 adds the confirmation for web links.
 */
export function OutlineTree({ items, currentPage, onJumpToPage }: OutlineTreeProps) {
  const [expanded, setExpanded] = useState(() => initiallyExpanded(items));
  const [focused, setFocused] = useState(0);
  const refs = useRef(new Map<number, HTMLLIElement>());

  const visible = visibleItems(items, expanded);
  const current = visibleAncestor(items, visible, currentItem(items, currentPage));
  // Focus lands on the item for the current page when the tree is entered.
  const tabStop = visible.includes(focused) ? focused : current >= 0 ? current : visible[0]!;

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
    const page = pageOf(items[index]!);
    if (page !== null) onJumpToPage(page + 1);
  };

  const onKeyDown = (event: KeyboardEvent, index: number) => {
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

  return (
    <ul role="tree" aria-label={t.outlineTab} className="space-y-0.5">
      {visible.map((index) => {
        const item = items[index]!;
        const parent = hasChildren(items, index);
        const open = expanded.has(index);
        const target = item.target;
        return (
          <li
            key={index}
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
            <span className="min-w-0 flex-1 truncate" title={item.title}>
              {item.title}
            </span>
            {target?.kind === "uri" && (
              <ExternalLink className="size-3.5 shrink-0 text-muted-foreground" role="img" aria-label={t.outlineExternalLink} />
            )}
            {target?.kind === "blocked" && (
              <Ban className="size-3.5 shrink-0 text-muted-foreground" role="img" aria-label={t.outlineBlockedAction} />
            )}
          </li>
        );
      })}
    </ul>
  );
}
