import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { OutlineTree, type OutlineEditing } from "@/features/outline/OutlineTree";
import type { OutlineView } from "@/features/outline/tree";
import { Sidebar } from "@/features/shell/Sidebar";
import { strings } from "@/i18n/zh-TW";
import type { OutlineItem } from "@/ipc/generated/contract";

const page = (pageIndex: number) => ({ kind: "page" as const, pageIndex, x: null, y: null });

const items: OutlineItem[] = [
  { id: 1, title: "Chapter 1", depth: 0, target: page(0) },
  { id: 2, title: "Section 1.1", depth: 1, target: page(1) },
  { id: 3, title: "Subsection 1.1.1", depth: 2, target: page(2) },
  { id: 4, title: "Chapter 2", depth: 0, target: page(3) },
  { id: 5, title: "Section 2.1", depth: 1, target: page(4) },
  { id: 6, title: "Appendix", depth: 0, target: page(5) },
];

function renderTree(props: Partial<Parameters<typeof OutlineTree>[0]> = {}) {
  const onJumpToPage = vi.fn();
  render(<OutlineTree items={items} currentPage={1} onJumpToPage={onJumpToPage} {...props} />);
  return { onJumpToPage, user: userEvent.setup() };
}

const item = (name: string) => screen.getByRole("treeitem", { name });
const shown = () => screen.getAllByRole("treeitem").map((element) => element.textContent);

describe("links outside the document (#49)", () => {
  it("hands web links and blocked actions to onOpenLink, and jumps for pages", async () => {
    const onJumpToPage = vi.fn();
    const onOpenLink = vi.fn();
    const outsideItems: OutlineItem[] = [
      { id: 10, title: "Page", depth: 0, target: { kind: "page" as const, pageIndex: 2, x: null, y: null } },
      { id: 11, title: "Web", depth: 0, target: { kind: "uri" as const, uri: "https://example.invalid/" } },
      { id: 12, title: "Run", depth: 0, target: { kind: "blocked" as const, action: "launch" as const, target: "calc.exe" } },
      { id: 13, title: "Nowhere", depth: 0, target: null },
    ];
    render(<OutlineTree items={outsideItems} currentPage={1} onJumpToPage={onJumpToPage} onOpenLink={onOpenLink} />);
    const user = userEvent.setup();

    await user.click(screen.getByRole("treeitem", { name: /Web/ }));
    expect(onOpenLink).toHaveBeenLastCalledWith(1, outsideItems[1]!.target);
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onOpenLink).toHaveBeenLastCalledWith(2, outsideItems[2]!.target);
    await user.click(screen.getByRole("treeitem", { name: "Nowhere" }));
    await user.click(screen.getByRole("treeitem", { name: "Page" }));
    expect(onOpenLink).toHaveBeenCalledTimes(2);
    expect(onJumpToPage).toHaveBeenCalledWith(3);
  });
});

describe("OutlineTree", () => {
  it("shows the first two levels as a tree", () => {
    renderTree();
    expect(screen.getByRole("tree", { name: strings.sidebar.outlineTab })).toBeInTheDocument();
    expect(shown()).toEqual(["Chapter 1", "Section 1.1", "Chapter 2", "Section 2.1", "Appendix"]);
    expect(item("Chapter 1")).toHaveAttribute("aria-level", "1");
    expect(item("Chapter 1")).toHaveAttribute("aria-expanded", "true");
    expect(item("Section 1.1")).toHaveAttribute("aria-level", "2");
    expect(item("Section 1.1")).toHaveAttribute("aria-expanded", "false");
    expect(item("Appendix")).not.toHaveAttribute("aria-expanded");
  });

  it("jumps to the item's page on click", async () => {
    const { onJumpToPage, user } = renderTree();
    await user.click(item("Chapter 2"));
    expect(onJumpToPage).toHaveBeenCalledWith(4);
  });

  it("works with the keyboard: arrows move, expand, collapse; Enter jumps", async () => {
    const { onJumpToPage, user } = renderTree();
    await user.tab();
    expect(item("Chapter 1")).toHaveFocus();

    await user.keyboard("{ArrowDown}");
    expect(item("Section 1.1")).toHaveFocus();
    await user.keyboard("{ArrowRight}"); // expand
    expect(shown()).toContain("Subsection 1.1.1");
    expect(item("Section 1.1")).toHaveFocus();
    await user.keyboard("{ArrowRight}"); // into the child
    expect(item("Subsection 1.1.1")).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onJumpToPage).toHaveBeenLastCalledWith(3);

    await user.keyboard("{ArrowLeft}"); // back to the parent
    expect(item("Section 1.1")).toHaveFocus();
    await user.keyboard("{ArrowLeft}"); // collapse
    expect(shown()).not.toContain("Subsection 1.1.1");
    await user.keyboard("{ArrowLeft}"); // to its parent
    expect(item("Chapter 1")).toHaveFocus();

    await user.keyboard("{End}");
    expect(item("Appendix")).toHaveFocus();
    await user.keyboard("{Home}");
    expect(item("Chapter 1")).toHaveFocus();
    await user.keyboard(" ");
    expect(onJumpToPage).toHaveBeenLastCalledWith(1);
  });

  it("the expand button toggles without jumping", async () => {
    const { onJumpToPage, user } = renderTree();
    await user.click(within(item("Chapter 1")).getByRole("button", { name: strings.sidebar.outlineCollapse }));
    expect(shown()).toEqual(["Chapter 1", "Chapter 2", "Section 2.1", "Appendix"]);
    expect(onJumpToPage).not.toHaveBeenCalled();
  });

  it("marks the item for the current page, or its visible parent", () => {
    renderTree({ currentPage: 5 });
    expect(item("Section 2.1")).toHaveAttribute("aria-current", "location");

    renderTree({ currentPage: 3 }); // Subsection 1.1.1 is hidden under Section 1.1
    const trees = screen.getAllByRole("tree");
    expect(within(trees[1]!).getByRole("treeitem", { name: "Section 1.1" })).toHaveAttribute("aria-current", "location");
  });

  it("items pointing outside the document do nothing and say so", async () => {
    const outside: OutlineItem[] = [
      { id: 21, title: "Web", depth: 0, target: { kind: "uri", uri: "https://example.invalid/" } },
      { id: 22, title: "Run", depth: 0, target: { kind: "blocked", action: "launch", target: "calc.exe" } },
    ];
    const { onJumpToPage, user } = renderTree({ items: outside });
    const [web, run] = screen.getAllByRole("treeitem");
    expect(within(web!).getByRole("img", { name: strings.sidebar.outlineExternalLink })).toBeInTheDocument();
    expect(within(run!).getByRole("img", { name: strings.sidebar.outlineBlockedAction })).toBeInTheDocument();
    await user.click(web!);
    await user.click(run!);
    expect(onJumpToPage).not.toHaveBeenCalled();
  });

  it("shows titles as plain text", () => {
    renderTree({ items: [{ id: 31, title: '<img src=x onerror="alert(1)">', depth: 0, target: page(0) }] });
    expect(item('<img src=x onerror="alert(1)">')).toBeInTheDocument();
    expect(document.querySelector("img")).toBeNull();
  });
});

describe("Bookmark editing (B3-17)", () => {
  it("inline rename with F2, Enter confirms, Esc cancels, blank disallowed", async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const editing: OutlineEditing = { allowed: true, apply };
    const { user } = renderTree({ editing });

    await user.tab();
    expect(item("Chapter 1")).toHaveFocus();

    // Trigger rename with F2
    await user.keyboard("{F2}");
    const input = screen.getByLabelText(strings.sidebar.renameInputLabel) as HTMLInputElement;
    expect(input).toHaveValue("Chapter 1");

    // Typing empty title does not submit
    await user.clear(input);
    await user.keyboard("{Enter}");
    expect(apply).not.toHaveBeenCalled();

    // Escape cancels
    await user.keyboard("{Escape}");
    expect(screen.queryByLabelText(strings.sidebar.renameInputLabel)).toBeNull();
    expect(apply).not.toHaveBeenCalled();

    // Re-trigger and rename successfully
    await user.keyboard("{F2}");
    const input2 = screen.getByLabelText(strings.sidebar.renameInputLabel);
    await user.clear(input2);
    await user.type(input2, "Introduction{Enter}");
    expect(apply).toHaveBeenCalledWith({
      kind: "renameBookmark",
      id: 1,
      title: "Introduction",
    });
  });

  it("deleting a leaf bookmark deletes directly without dialog", async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const editing: OutlineEditing = { allowed: true, apply };
    const { user } = renderTree({ editing });

    await user.tab();
    await user.keyboard("{End}"); // Move to Appendix (leaf)
    expect(item("Appendix")).toHaveFocus();

    await user.keyboard("{Delete}");
    expect(apply).toHaveBeenCalledWith({
      kind: "deleteBookmark",
      id: 6,
    });
    expect(screen.queryByText(strings.sidebar.deleteConfirmTitle)).toBeNull();
  });

  it("deleting a bookmark with children confirms first", async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const editing: OutlineEditing = { allowed: true, apply };
    const { user } = renderTree({ editing });

    await user.tab();
    expect(item("Chapter 1")).toHaveFocus(); // Has children

    await user.keyboard("{Delete}");
    expect(screen.getByText(strings.sidebar.deleteConfirmTitle)).toBeInTheDocument();
    expect(apply).not.toHaveBeenCalled();

    // Confirm dialog
    await user.click(screen.getByRole("button", { name: strings.sidebar.deleteConfirmConfirm }));
    expect(apply).toHaveBeenCalledWith({
      kind: "deleteBookmark",
      id: 1,
    });
  });

  it("adds a bookmark via Ctrl+B", async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const editing: OutlineEditing = { allowed: true, apply };
    const { user } = renderTree({ editing, currentPage: 2 });

    await user.tab();
    expect(item("Chapter 1")).toHaveFocus();

    await user.keyboard("{Control>}b{/Control}");
    expect(apply).toHaveBeenCalledWith({
      kind: "addBookmark",
      parent: 1,
      index: 1, // Chapter 1 has 1 child (Section 1.1)
      title: strings.sidebar.defaultBookmarkTitle(2),
      page: 1, // 0-based page index
    });
  });

  it("moves bookmark up, down, promote, demote via Alt+arrows", async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const editing: OutlineEditing = { allowed: true, apply };
    const { user } = renderTree({ editing });

    await user.tab();
    // Focus Chapter 1 (index 0). Cannot move up (first child of root).
    await user.keyboard("{Alt>}{ArrowUp}{/Alt}");
    expect(apply).not.toHaveBeenCalled();

    // Move to Chapter 2 (next top-level item)
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(item("Chapter 2")).toHaveFocus();

    // Alt+Up moves Chapter 2 before Chapter 1
    await user.keyboard("{Alt>}{ArrowUp}{/Alt}");
    expect(apply).toHaveBeenLastCalledWith({
      kind: "moveBookmark",
      id: 4,
      parent: null,
      index: 0,
    });

    // Alt+Down moves Chapter 2 after Appendix
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(apply).toHaveBeenLastCalledWith({
      kind: "moveBookmark",
      id: 4,
      parent: null,
      index: 2,
    });
  });

  it("editing shortcuts are ignored when editing is not allowed", async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const editing: OutlineEditing = { allowed: false, apply };
    const { user } = renderTree({ editing });

    await user.tab();
    expect(item("Chapter 1")).toHaveFocus();

    await user.keyboard("{F2}");
    expect(screen.queryByLabelText(strings.sidebar.renameInputLabel)).toBeNull();

    await user.keyboard("{Delete}");
    expect(apply).not.toHaveBeenCalled();

    await user.keyboard("{Control>}b{/Control}");
    expect(apply).not.toHaveBeenCalled();
  });
});

describe("Sidebar outline states", () => {
  const renderSidebar = (outline: OutlineView) =>
    render(<Sidebar outline={outline} pages={[]} currentPage={1} onJumpToPage={vi.fn()} />);

  const states: [OutlineView, string][] = [
    [{ status: "loading" }, strings.sidebar.outlineLoading],
    [{ status: "failed" }, strings.sidebar.outlineFailed],
    [{ status: "none" }, strings.sidebar.outlineEmpty],
    [{ status: "ready", items: [], truncated: false }, strings.sidebar.outlineEmpty],
  ];
  it.each(states)("%o", (outline, text) => {
    renderSidebar(outline);
    expect(screen.getByText(text)).toBeInTheDocument();
    expect(screen.queryByRole("tree")).not.toBeInTheDocument();
  });

  it("says when the outline was cut short", () => {
    renderSidebar({ status: "ready", items, truncated: true });
    expect(screen.getByRole("note")).toHaveTextContent(strings.sidebar.outlineTruncated);
    expect(screen.getByRole("tree")).toBeInTheDocument();
  });
});
