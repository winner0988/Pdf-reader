import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/tooltip";
import type { Tab } from "@/features/tabs/model";
import { TabBar } from "@/features/tabs/TabBar";
import { strings } from "@/i18n/zh-TW";

const tabs: Tab[] = [
  { tab: 1, displayName: "a.pdf", content: { kind: "loading" } },
  { tab: 2, displayName: "b.pdf", content: { kind: "error", code: "notPdf" } },
  { tab: 3, displayName: "c.pdf", content: { kind: "loading" } },
];

function renderBar(active = 2) {
  const handlers = { onActivate: vi.fn(), onClose: vi.fn(), onOpen: vi.fn() };
  render(
    <TooltipProvider>
      <TabBar tabs={tabs} active={active} {...handlers} />
    </TooltipProvider>,
  );
  return { ...handlers, user: userEvent.setup() };
}

describe("TabBar", () => {
  it("is a tab list whose shown tab is selected and focusable", () => {
    renderBar();
    const list = screen.getByRole("tablist", { name: strings.tabs.label });
    const all = within(list).getAllByRole("tab");
    expect(all.map((tab) => tab.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);
    expect(all.map((tab) => tab.tabIndex)).toEqual([-1, 0, -1]);
    // Loading and failed tabs say so to screen readers.
    expect(all[0]).toHaveAccessibleName(`${strings.tabs.loading}a.pdf`);
    expect(all[1]).toHaveAccessibleName(`${strings.tabs.failed}b.pdf`);
  });

  it("moves between tabs with the arrow keys, Home and End", async () => {
    const { onActivate, user } = renderBar();
    await user.click(screen.getByRole("tab", { selected: true }));
    onActivate.mockClear();

    // Focus follows: after ArrowRight the third tab has it, so ArrowLeft goes back to the second.
    await user.keyboard("{ArrowRight}");
    await user.keyboard("{ArrowLeft}");
    await user.keyboard("{Home}");
    await user.keyboard("{End}");
    expect(onActivate.mock.calls.map(([tab]) => tab)).toEqual([3, 2, 1, 3]);
    expect(screen.getByRole("tab", { name: /c\.pdf/ })).toHaveFocus();
  });

  it("closes a tab with its button or a middle click, and opens files with the plus button", async () => {
    const { onClose, onOpen, user } = renderBar();
    await user.click(screen.getByRole("button", { name: strings.tabs.close("c.pdf") }));
    expect(onClose).toHaveBeenLastCalledWith(3);

    await user.pointer({ keys: "[MouseMiddle]", target: screen.getByRole("tab", { name: /a\.pdf/ }) });
    expect(onClose).toHaveBeenLastCalledWith(1);

    await user.click(screen.getByRole("button", { name: strings.tabs.open }));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });
});
