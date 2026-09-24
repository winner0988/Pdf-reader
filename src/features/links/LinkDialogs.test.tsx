import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { BlockedLinkDialog, LinkConfirmDialog } from "@/features/links/LinkDialogs";
import { strings } from "@/i18n/zh-TW";
import type { BlockedAction, LinkPreview } from "@/ipc/generated/contract";

const t = strings.links;

const web = (uri: string, more: Partial<LinkPreview> = {}): LinkPreview => ({
  uri,
  opens: uri,
  host: "example.invalid",
  asciiHost: null,
  ...more,
});

function confirm(preview: LinkPreview) {
  const onOpen = vi.fn(() => Promise.resolve());
  const onClose = vi.fn();
  render(<LinkConfirmDialog preview={preview} onOpen={onOpen} onClose={onClose} />);
  const dialog = screen.getByRole("dialog", { name: t.confirmTitle });
  return { dialog, onOpen, onClose, fullUrl: within(dialog).getByLabelText(t.confirmFullUrl) };
}

// user-event's setup() puts a clipboard stub on navigator; copied text is read back from it.

describe("LinkConfirmDialog", () => {
  it("names the site in bold and shows the whole URL, with no warnings for an ordinary link", () => {
    const { dialog, fullUrl } = confirm(web("https://example.invalid/docs?q=1"));
    expect(within(dialog).getByText("example.invalid").tagName).toBe("STRONG");
    expect(fullUrl).toHaveTextContent("https://example.invalid/docs?q=1");
    expect(dialog).not.toHaveTextContent("可能是假冒");
    expect(dialog).not.toHaveTextContent("隱藏字元");
  });

  it("warns about a look-alike name and gives its real, punycode form", () => {
    const { dialog } = confirm(
      web("https://аpple.example.invalid/", {
        host: "аpple.example.invalid",
        asciiHost: "xn--pple-43d.example.invalid",
      }),
    );
    expect(within(dialog).getByText("аpple.example.invalid")).toBeInTheDocument();
    expect(dialog).toHaveTextContent(t.warnIdn("xn--pple-43d.example.invalid"));
  });

  it("writes out hidden characters and warns about them", () => {
    const { dialog, fullUrl } = confirm(web("https://example.invalid/‮fdp.exe"));
    expect(fullUrl.textContent).toBe("https://example.invalid/[U+202E]fdp.exe");
    expect(dialog).toHaveTextContent(t.warnControl);
    // Both warnings can show at once.
    const both = web("https://аpple.example.invalid/‮", { asciiHost: "xn--pple-43d.example.invalid" });
    const { dialog: second } = (() => {
      render(<LinkConfirmDialog preview={both} onOpen={vi.fn()} onClose={vi.fn()} />);
      return { dialog: screen.getAllByRole("dialog").at(-1)! };
    })();
    expect(second).toHaveTextContent(t.warnControl);
    expect(second).toHaveTextContent("xn--pple-43d.example.invalid");
  });

  it("never cuts a long URL", () => {
    const long = `https://example.invalid/${"a".repeat(9_976)}`;
    const { fullUrl } = confirm(web(long));
    expect(fullUrl.textContent).toHaveLength(10_000);
  });

  it("copies what would be opened, and opens only on 開啟", async () => {
    const user = userEvent.setup();
    const { dialog, onOpen, onClose } = confirm(
      web("https://example.invalid/‮x", { opens: "https://example.invalid/%E2%80%AEx" }),
    );
    await user.click(within(dialog).getByRole("button", { name: t.copy }));
    expect(await navigator.clipboard.readText()).toBe("https://example.invalid/%E2%80%AEx");
    expect(onOpen).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: t.open }));
    expect(onOpen).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("has no site line for a link without one", () => {
    const { dialog } = confirm(web("mailto:someone", { host: null }));
    expect(dialog).not.toHaveTextContent(`${t.confirmHost}：`);
  });
});

describe("BlockedLinkDialog", () => {
  it("explains each reason and shows the content for reading only", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<BlockedLinkDialog action="networkShare" content={"\\\\share.example.invalid\\x"} onClose={onClose} />);
    const dialog = screen.getByRole("dialog", { name: t.blockedTitle });
    expect(dialog).toHaveTextContent(t.blocked.networkShare.description);
    expect(within(dialog).getByLabelText(t.blockedContent)).toHaveTextContent("\\\\share.example.invalid\\x");
    await user.click(within(dialog).getByRole("button", { name: t.blockedCopy }));
    expect(await navigator.clipboard.readText()).toBe("\\\\share.example.invalid\\x");
    await user.click(within(dialog).getByRole("button", { name: t.close }));
    expect(onClose).toHaveBeenCalled();
  });

  it("has a text for every reason, and says when there is no content", () => {
    const actions: BlockedAction[] = [
      "launch",
      "remoteGoTo",
      "embeddedGoTo",
      "javaScript",
      "submitForm",
      "importData",
      "localFile",
      "networkShare",
      "other",
    ];
    for (const action of actions) expect(t.blocked[action].description.length).toBeGreaterThan(10);
    render(<BlockedLinkDialog action="embeddedGoTo" content={null} onClose={vi.fn()} />);
    const dialog = screen.getByRole("dialog", { name: t.blockedTitle });
    expect(within(dialog).getByLabelText(t.blockedContent)).toHaveTextContent(t.blockedNoContent);
    expect(within(dialog).queryByRole("button", { name: t.blockedCopy })).not.toBeInTheDocument();
  });
});
