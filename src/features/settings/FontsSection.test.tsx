import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FontsSection, type FontsApi } from "@/features/settings/FontsSection";
import { strings } from "@/i18n/zh-TW";
import type { FontId, FontImport, FontImportRefusal, FontInfo } from "@/ipc/generated/contract";

const t = strings.settings.fonts;

const FONT_INSTALLED: FontInfo = {
  id: 1 as FontId,
  name: "Arial",
  family: "Arial",
  bold: false,
  italic: false,
  source: "installed",
};

const FONT_IMPORTED_1: FontInfo = {
  id: 2 as FontId,
  name: "Custom Sans",
  family: "Custom Sans",
  bold: false,
  italic: false,
  source: "imported",
};

const FONT_IMPORTED_2: FontInfo = {
  id: 3 as FontId,
  name: "Custom Serif",
  family: "Custom Serif",
  bold: false,
  italic: false,
  source: "imported",
};

function fakeApi(initial: FontInfo[]) {
  let current = [...initial];
  return {
    getFonts: vi.fn<FontsApi["getFonts"]>(() => Promise.resolve([...current])),
    importFont: vi.fn<FontsApi["importFont"]>(() =>
      Promise.resolve<FontImport>({ kind: "cancelled" }),
    ),
    removeFont: vi.fn<FontsApi["removeFont"]>((id) => {
      current = current.filter((f) => f.id !== id);
      return Promise.resolve([...current]);
    }),
    removeAllFonts: vi.fn<FontsApi["removeAllFonts"]>(() => {
      current = current.filter((f) => f.source !== "imported");
      return Promise.resolve([...current]);
    }),
  } satisfies FontsApi;
}

describe("the custom fonts section in the settings (B2-32)", () => {
  it("renders title, note and empty state when no font is imported", async () => {
    const api = fakeApi([FONT_INSTALLED]);
    render(<FontsSection api={api} />);

    expect(await screen.findByRole("heading", { name: t.title })).toBeVisible();
    expect(screen.getByText(t.note)).toBeVisible();
    expect(screen.getByText(t.noneImported)).toBeVisible();
    expect(screen.getByRole("button", { name: t.importButton })).toBeVisible();
    expect(screen.queryByRole("button", { name: t.removeAll })).not.toBeInTheDocument();
  });

  it("lists imported fonts and hides installed ones from the imported list", async () => {
    const api = fakeApi([FONT_INSTALLED, FONT_IMPORTED_1, FONT_IMPORTED_2]);
    render(<FontsSection api={api} />);

    expect(await screen.findByText(FONT_IMPORTED_1.name)).toBeVisible();
    expect(screen.getByText(FONT_IMPORTED_2.name)).toBeVisible();
    // Installed font is not listed under imported fonts
    const list = screen.getByRole("list", { name: t.importedFonts });
    expect(list).toHaveTextContent(FONT_IMPORTED_1.name);
    expect(list).toHaveTextContent(FONT_IMPORTED_2.name);
    expect(list).not.toHaveTextContent(FONT_INSTALLED.name);
  });

  it("imports a font successfully and updates the list", async () => {
    const api = fakeApi([FONT_INSTALLED]);
    api.importFont.mockResolvedValueOnce({
      kind: "imported",
      fonts: [FONT_INSTALLED, FONT_IMPORTED_1],
    });
    const user = userEvent.setup();
    render(<FontsSection api={api} />);

    await user.click(await screen.findByRole("button", { name: t.importButton }));
    expect(api.importFont).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(FONT_IMPORTED_1.name)).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent(t.imported);
  });

  it("shows refusal reason when import is refused", async () => {
    const reasons: FontImportRefusal[] = [
      "badExtension",
      "tooLarge",
      "tooMany",
      "notAFont",
      "notTrueType",
      "variable",
      "colour",
      "notEmbeddable",
      "unreadable",
    ];

    for (const reason of reasons) {
      const api = fakeApi([FONT_INSTALLED]);
      api.importFont.mockResolvedValueOnce({ kind: "refused", reason });
      const user = userEvent.setup();
      const { unmount } = render(<FontsSection api={api} />);

      await user.click(await screen.findByRole("button", { name: t.importButton }));
      expect(screen.getByRole("status")).toHaveTextContent(t.refused[reason]);
      unmount();
    }
  });

  it("handles cancelled import without showing notice", async () => {
    const api = fakeApi([FONT_INSTALLED]);
    api.importFont.mockResolvedValueOnce({ kind: "cancelled" });
    const user = userEvent.setup();
    render(<FontsSection api={api} />);

    await user.click(await screen.findByRole("button", { name: t.importButton }));
    expect(screen.getByRole("status")).toHaveTextContent("");
  });

  it("removes a single imported font", async () => {
    const api = fakeApi([FONT_INSTALLED, FONT_IMPORTED_1]);
    const user = userEvent.setup();
    render(<FontsSection api={api} />);

    const removeBtn = await screen.findByRole("button", { name: t.remove(FONT_IMPORTED_1.name) });
    await user.click(removeBtn);

    expect(api.removeFont).toHaveBeenCalledWith(FONT_IMPORTED_1.id);
    expect(screen.queryByText(FONT_IMPORTED_1.name)).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(t.removed);
  });

  it("clears all imported fonts after user confirmation", async () => {
    const api = fakeApi([FONT_INSTALLED, FONT_IMPORTED_1, FONT_IMPORTED_2]);
    const user = userEvent.setup();
    render(<FontsSection api={api} />);

    await user.click(await screen.findByRole("button", { name: t.removeAll }));
    const dialog = screen.getByRole("group", { name: t.confirmRemoveAllTitle });
    expect(dialog).toHaveTextContent(t.confirmRemoveAllText);

    // Cancel first
    await user.click(screen.getByRole("button", { name: t.cancel }));
    expect(api.removeAllFonts).not.toHaveBeenCalled();
    expect(screen.queryByRole("group", { name: t.confirmRemoveAllTitle })).not.toBeInTheDocument();

    // Confirm remove all
    await user.click(screen.getByRole("button", { name: t.removeAll }));
    await user.click(screen.getByRole("button", { name: t.confirmRemoveAll }));

    expect(api.removeAllFonts).toHaveBeenCalledTimes(1);
    expect(screen.queryByText(FONT_IMPORTED_1.name)).not.toBeInTheDocument();
    expect(screen.queryByText(FONT_IMPORTED_2.name)).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(t.allRemoved);
  });

  it("shows error alert when getFonts fails", async () => {
    const api = fakeApi([]);
    api.getFonts.mockRejectedValueOnce(new Error("failed"));
    render(<FontsSection api={api} />);

    expect(await screen.findByRole("alert")).toHaveTextContent(t.failed);
  });
});
