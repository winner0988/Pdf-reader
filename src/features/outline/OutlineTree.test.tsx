import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { OutlineTree } from "@/features/outline/OutlineTree";
import type { OutlineView } from "@/features/outline/tree";
import { Sidebar } from "@/features/shell/Sidebar";
import { strings } from "@/i18n/zh-TW";
import type { OutlineItem } from "@/ipc/generated/contract";

const page = (pageIndex: number) => ({ kind: "page" as const, pageIndex, x: null, y: null });

const items: OutlineItem[] = [
  { title: "Chapter 1", depth: 0, target: page(0) },
  { title: "Section 1.1", depth: 1, target: page(1) },
  { title: "Subsection 1.1.1", depth: 2, target: page(2) },
  { title: "Chapter 2", depth: 0, target: page(3) },
  { title: "Section 2.1", depth: 1, target: page(4) },
  { title: "Appendix", depth: 0, target: page(5) },
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
    const items = [
      { title: "Page", depth: 0, target: { kind: "page" as const, pageIndex: 2, x: null, y: null } },
      { title: "Web", depth: 0, target: { kind: "uri" as const, uri: "https://example.invalid/" } },
      { title: "Run", depth: 0, target: { kind: "blocked" as const, action: "launch" as const, target: "calc.exe" } },
      { title: "Nowhere", depth: 0, target: null },
    ];
    render(<OutlineTree items={items} currentPage={1} onJumpToPage={onJumpToPage} onOpenLink={onOpenLink} />);
    const user = userEvent.setup();

    await user.click(screen.getByRole("treeitem", { name: /Web/ }));
    expect(onOpenLink).toHaveBeenLastCalledWith(1, items[1]!.target);
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onOpenLink).toHaveBeenLastCalledWith(2, items[2]!.target);
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
      { title: "Web", depth: 0, target: { kind: "uri", uri: "https://example.invalid/" } },
      { title: "Run", depth: 0, target: { kind: "blocked", action: "launch", target: "calc.exe" } },
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
    renderTree({ items: [{ title: '<img src=x onerror="alert(1)">', depth: 0, target: page(0) }] });
    expect(item('<img src=x onerror="alert(1)">')).toBeInTheDocument();
    expect(document.querySelector("img")).toBeNull();
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
