import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { LinksApi } from "@/features/links/source";
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

  it("asking for a password that can be remembered, and when the remembered one did not fit (B2-26)", async () => {
    for (const rememberedFailed of [false, true]) {
      const { container, unmount } = shell({
        kind: "password",
        displayName: "機密.pdf",
        wrong: false,
        canRemember: true,
        rememberedFailed,
      });
      await expectAccessible(container);
      unmount();
    }
    // A file whose password cannot be remembered says why.
    const { container } = shell({
      kind: "password",
      displayName: "機密.pdf",
      wrong: true,
      canRemember: false,
      rememberedFailed: false,
    });
    await expectAccessible(container);
  });

  it("a file that opened with the password the app remembered (B2-26)", async () => {
    const { container } = render(
      <ReaderShell
        state={{ kind: "open", document: { ...demoDocument, encrypted: true, passwordRemembered: true, passwordRecalled: true } }}
        onOpen={vi.fn()}
        onForgetPassword={vi.fn()}
        loadingDelayMs={0}
      />,
    );
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

  it("the thumbnails of a sensitive document (B3-10)", async () => {
    shell({ kind: "open", document: { ...demoDocument, sensitive: true } });
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));
    expect(screen.getByText(strings.sidebar.thumbnailsSensitive)).toBeInTheDocument();
    await expectAccessible(document.body);
  });

  it("the details of what was blocked", async () => {
    shell({ kind: "open", document: demoDocument });
    await user.click(screen.getByRole("button", { name: strings.banner.details }));
    await expectAccessible(document.body);
  });

  it("the details of what was blocked, with web addresses to open in the browser (B3-08)", async () => {
    const linksApi = {
      getPageLinks: vi.fn(() => Promise.resolve([])),
      describeLink: vi.fn(),
      openLink: vi.fn(),
      describeOutlineLink: vi.fn(),
      openOutlineLink: vi.fn(),
      describeRemoteAddress: vi.fn(),
      openRemoteAddress: vi.fn(),
    } satisfies LinksApi;
    const opened = {
      ...demoDocument,
      doc: 1,
      remoteAddresses: ["https://img.example.invalid/beacon.png", "https://a.example.invalid/‮gnp.exe"],
    };
    render(
      <ReaderShell state={{ kind: "open", document: opened }} onOpen={vi.fn()} loadingDelayMs={0} linksApi={linksApi} />,
    );
    await user.click(screen.getByRole("button", { name: strings.banner.details }));
    expect(screen.getAllByRole("button", { name: strings.banner.openAddress })).toHaveLength(2);
    await expectAccessible(document.body);
  });

  it("the settings", async () => {
    shell({ kind: "open", document: demoDocument });
    await chooseFromMenu(strings.menu.settings);
    await screen.findByRole("dialog", { name: strings.settings.title });
    await expectAccessible(document.body);
  });

  it("the remembered passwords in the settings, and the question before forgetting them all (B2-26)", async () => {
    const passwordsApi = {
      rememberedPasswords: vi.fn(() => Promise.resolve({ count: 3, limit: 256 })),
      forgetAllPasswords: vi.fn(() => Promise.resolve(3)),
    };
    render(<ReaderShell state={{ kind: "open", document: demoDocument }} onOpen={vi.fn()} passwordsApi={passwordsApi} loadingDelayMs={0} />);
    await chooseFromMenu(strings.menu.settings);
    await screen.findByRole("dialog", { name: strings.settings.title });
    await screen.findByText(strings.settings.passwords.count(3));
    await expectAccessible(document.body);
    await user.click(screen.getByRole("button", { name: strings.settings.passwords.forgetAll }));
    await screen.findByRole("group", { name: strings.settings.passwords.confirmTitle });
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
