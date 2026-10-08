import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { SOURCE_URL } from "@/features/shell/about";
import { AboutDialog } from "@/features/shell/dialogs";
import { strings } from "@/i18n/zh-TW";

const t = strings.about;

function setup(onOpenChange = vi.fn()) {
  const view = render(<AboutDialog open onOpenChange={onOpenChange} version="1.2.3" />);
  return { ...view, onOpenChange, user: userEvent.setup() };
}

describe("about (ADR 0011)", () => {
  it("says what the licence is, where the source is and which tag of it this version is", () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: t.title });
    expect(within(dialog).getByText(t.version("1.2.3"))).toBeInTheDocument();
    expect(within(dialog).getByText(t.license)).toBeInTheDocument();
    expect(within(dialog).getByText(SOURCE_URL)).toBeInTheDocument();
    expect(within(dialog).getByText(t.sourceTag("1.2.3"))).toHaveTextContent("v1.2.3");
    // Text, not a link: following one is the user's decision, through the link confirmation.
    expect(within(dialog).queryByRole("link")).toBeNull();
  });

  it("shows the licences of the components, from the app's own files, and goes back", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: t.thirdParty }));
    const dialog = screen.getByRole("dialog", { name: strings.licenses.title });
    const text = await within(dialog).findByRole("region", { name: strings.licenses.textLabel });
    // MuPDF and what is built into it, the Rust crates, the JavaScript packages, and the texts.
    for (const part of ["MuPDF", "FreeType", "Tesseract", "Rust crates", "JavaScript packages", "GNU AFFERO GENERAL PUBLIC LICENSE"]) {
      expect(text.textContent).toContain(part);
    }
    // The text can be scrolled with the keyboard.
    expect(text).toHaveAttribute("tabindex", "0");
    await user.click(within(dialog).getByRole("button", { name: strings.licenses.back }));
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();
  });

  it("opens on the about page again after it was closed on the licences", async () => {
    const { user, rerender, onOpenChange } = setup();
    await user.click(screen.getByRole("button", { name: t.thirdParty }));
    await screen.findByRole("region", { name: strings.licenses.textLabel });
    await user.keyboard("{Escape}");
    expect(onOpenChange).toHaveBeenCalledWith(false);
    rerender(<AboutDialog open={false} onOpenChange={onOpenChange} version="1.2.3" />);
    rerender(<AboutDialog open onOpenChange={onOpenChange} version="1.2.3" />);
    expect(await screen.findByRole("dialog", { name: t.title })).toBeInTheDocument();
  });
});
