import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import { expectAccessible } from "@/test/axe";

function shell(state: ShellState) {
  return render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} />);
}

describe("the window's states", () => {
  it("empty", async () => {
    const { container } = shell({ kind: "empty" });
    await expectAccessible(container);
  });

  it("loading", async () => {
    const { container } = shell({ kind: "loading", displayName: "報告.pdf" });
    await expectAccessible(container);
  });

  it("an error", async () => {
    const { container } = shell({ kind: "error", code: "corrupted", displayName: "報告.pdf" });
    await expectAccessible(container);
  });

  it("asking for a password", async () => {
    const { container } = shell({ kind: "password", displayName: "機密.pdf", wrong: false });
    await expectAccessible(container);
  });

  it("an open document", async () => {
    const { container } = shell({ kind: "open", document: demoDocument });
    await expectAccessible(container);
  });
});

describe("the check itself", () => {
  it("fails, and names the element, when a button has no name", async () => {
    const { container } = render(
      <button type="button">
        <svg aria-hidden="true" />
      </button>,
    );
    await expect(expectAccessible(container)).rejects.toThrow(/button-name/);
  });
});

describe("the window's menus and bars", () => {
  const user = userEvent.setup();

  it("the menu of more actions", async () => {
    shell({ kind: "open", document: demoDocument });
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await screen.findByRole("menu");
    await expectAccessible(document.body, {
      skip: { region: "a menu is a pop-up, outside the page's landmarks by design" },
    });
  });

  it("the search bar", async () => {
    shell({ kind: "open", document: demoDocument });
    await user.click(screen.getByRole("button", { name: strings.toolbar.search }));
    await screen.findByRole("search");
    await expectAccessible(document.body);
  });
});

describe("the window's panels and the dialogs of its menu", () => {
  const user = userEvent.setup();

  /** An item's name is followed by its shortcut, so the name is matched at the start. */
  async function chooseFromMenu(name: string) {
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(`^${name}`) }));
  }

  it("the thumbnails", async () => {
    shell({ kind: "open", document: demoDocument });
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));
    await expectAccessible(document.body);
  });

  it("the details of what was blocked", async () => {
    shell({ kind: "open", document: demoDocument });
    await user.click(screen.getByRole("button", { name: strings.banner.details }));
    await expectAccessible(document.body);
  });

  it("the settings", async () => {
    shell({ kind: "open", document: demoDocument });
    await chooseFromMenu(strings.menu.settings);
    await screen.findByRole("dialog", { name: strings.settings.title });
    await expectAccessible(document.body);
  });

  it("the shortcuts", async () => {
    shell({ kind: "open", document: demoDocument });
    await chooseFromMenu(strings.menu.shortcuts);
    await screen.findByRole("dialog", { name: strings.shortcuts.title });
    await expectAccessible(document.body);
  });

  it("about", async () => {
    shell({ kind: "open", document: demoDocument });
    await chooseFromMenu(strings.menu.about);
    await screen.findByRole("dialog", { name: strings.about.title });
    await expectAccessible(document.body);
    // The licences of the components, which the page shows when asked.
    await user.click(screen.getByRole("button", { name: strings.about.thirdParty }));
    await screen.findByRole("region", { name: strings.licenses.textLabel });
    await expectAccessible(document.body);
  });
});
