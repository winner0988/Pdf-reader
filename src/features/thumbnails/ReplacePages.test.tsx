import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellDocument, ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";
import type { PagesSource } from "@/ipc/generated/contract";

const t = strings.pages;
const taken: PagesSource = { source: 7, pages: 3 };

function fakeEditingApi() {
  return {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
    pickPagesSource: vi.fn<NonNullable<EditingApi["pickPagesSource"]>>(() => Promise.resolve(taken)),
    unlockPagesSource: vi.fn<NonNullable<EditingApi["unlockPagesSource"]>>(() => Promise.resolve(taken)),
  } satisfies EditingApi;
}

function renderShell(document: Partial<ShellDocument> = {}) {
  const api = fakeEditingApi();
  const state = (overrides: Partial<ShellDocument>): ShellState => ({
    kind: "open",
    document: { ...demoDocument, doc: 5, session: 1, ...document, ...overrides },
  });
  const view = render(<ReaderShell state={state({})} onOpen={vi.fn()} loadingDelayMs={0} editingApi={api} />);
  const rerender = (overrides: Partial<ShellDocument>) =>
    view.rerender(<ReaderShell state={state(overrides)} onOpen={vi.fn()} loadingDelayMs={0} editingApi={api} />);
  return { api, rerender, user: userEvent.setup() };
}

describe("replace pages with another file (B6-05, #590)", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  });

  it("shows error and does not pick source when selection is non-contiguous", async () => {
    const { api, user } = renderShell();
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));

    // Select page 2
    await user.click(await screen.findByRole("option", { name: strings.canvas.page(2) }));
    // Ctrl-click page 4 (non-contiguous)
    await user.keyboard("{Control>}");
    await user.click(await screen.findByRole("option", { name: strings.canvas.page(4) }));
    await user.keyboard("{/Control}");

    // Right-click page 4 to open context menu
    fireEvent.contextMenu(screen.getByRole("option", { name: strings.canvas.page(4) }));
    const menuItem = await screen.findByRole("menuitem", { name: new RegExp(`^${t.replacePages}`) });
    await user.click(menuItem);

    // Shows alert saying only contiguous pages can be replaced
    expect(await screen.findByRole("alert")).toHaveTextContent(t.replaceOnlyContiguous);
    expect(api.pickPagesSource).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog", { name: t.replace.title })).toBeNull();
  });

  it("opens dialog with default range and applies edit when contiguous pages are selected", async () => {
    const { api, user } = renderShell();
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));

    // Select page 2
    await user.click(await screen.findByRole("option", { name: strings.canvas.page(2) }));
    // Shift-click page 3 (contiguous pages 2 and 3)
    await user.keyboard("{Shift>}");
    await user.click(await screen.findByRole("option", { name: strings.canvas.page(3) }));
    await user.keyboard("{/Shift}");

    // Right-click page 3
    fireEvent.contextMenu(screen.getByRole("option", { name: strings.canvas.page(3) }));
    const menuItem = await screen.findByRole("menuitem", { name: new RegExp(`^${t.replacePages}`) });
    await user.click(menuItem);

    expect(api.pickPagesSource).toHaveBeenCalledWith(5);

    // Dialog opens
    const dialog = await screen.findByRole("dialog", { name: t.replace.title });
    expect(dialog).toBeInTheDocument();

    // Default range is 1 to 3 (source has 3 pages)
    const fromInput = within(dialog).getByLabelText(t.replace.from);
    const toInput = within(dialog).getByLabelText(t.replace.to);
    expect(fromInput).toHaveValue(1);
    expect(toInput).toHaveValue(3);

    // Preview shows total after replacing 2 pages with 3 pages (demoDocument has 12 pages: 12 - 2 + 3 = 13 pages)
    expect(within(dialog).getByText(t.replace.previewTitle(13))).toBeInTheDocument();

    // Submit dialog
    await user.click(within(dialog).getByRole("button", { name: t.replace.confirm }));

    await waitFor(() =>
      expect(api.applyEdit).toHaveBeenCalledWith(5, {
        kind: "replacePages",
        first: 1, // 0-based index of page 2
        count: 2, // 2 pages
        source: 7,
        from: 0,
        to: 2,
      }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.replace.title })).toBeNull());
  });

  it("updates preview when range changes and validates range order", async () => {
    const { api, user } = renderShell();
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));

    // Select page 2
    await user.click(await screen.findByRole("option", { name: strings.canvas.page(2) }));
    fireEvent.contextMenu(screen.getByRole("option", { name: strings.canvas.page(2) }));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(`^${t.replacePages}`) }));

    const dialog = await screen.findByRole("dialog", { name: t.replace.title });
    const fromInput = within(dialog).getByLabelText(t.replace.from);
    const toInput = within(dialog).getByLabelText(t.replace.to);
    const confirmButton = within(dialog).getByRole("button", { name: t.replace.confirm });

    // Change from to 2, to to 2 (1 source page replacing 1 doc page -> 12 - 1 + 1 = 12 pages)
    await user.clear(fromInput);
    await user.type(fromInput, "2");
    await user.clear(toInput);
    await user.type(toInput, "2");

    expect(within(dialog).getByText(t.replace.previewTitle(12))).toBeInTheDocument();

    // Set invalid range (from 3, to 1 -> from > to)
    await user.clear(fromInput);
    await user.type(fromInput, "3");
    await user.clear(toInput);
    await user.type(toInput, "1");

    expect(within(dialog).getByRole("alert")).toHaveTextContent(t.replace.fromGreaterThanTo);
    expect(confirmButton).toBeDisabled();

    // Fix range: from 1, to 2
    await user.clear(fromInput);
    await user.type(fromInput, "1");
    await user.clear(toInput);
    await user.type(toInput, "2");
    expect(confirmButton).not.toBeDisabled();

    await user.click(confirmButton);
    await waitFor(() =>
      expect(api.applyEdit).toHaveBeenCalledWith(5, {
        kind: "replacePages",
        first: 1,
        count: 1,
        source: 7,
        from: 0,
        to: 1,
      }),
    );
  });

  it("does not apply edit when dialog is canceled", async () => {
    const { api, user } = renderShell();
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));

    await user.click(await screen.findByRole("option", { name: strings.canvas.page(1) }));
    fireEvent.contextMenu(screen.getByRole("option", { name: strings.canvas.page(1) }));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(`^${t.replacePages}`) }));

    const dialog = await screen.findByRole("dialog", { name: t.replace.title });
    await user.click(within(dialog).getByRole("button", { name: t.replace.cancel }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.replace.title })).toBeNull());
    expect(api.applyEdit).not.toHaveBeenCalled();
  });

  it("prompts for password on encrypted source before opening replace dialog", async () => {
    const { api, user } = renderShell();
    api.pickPagesSource.mockRejectedValue({ code: "encrypted", message: "" });
    api.unlockPagesSource.mockImplementation((_doc, password) =>
      password === "secret" ? Promise.resolve(taken) : Promise.reject({ code: "encrypted", message: "" }),
    );

    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));
    await user.click(await screen.findByRole("option", { name: strings.canvas.page(1) }));
    fireEvent.contextMenu(screen.getByRole("option", { name: strings.canvas.page(1) }));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(`^${t.replacePages}`) }));

    // Source password dialog prompts
    const pwdDialog = await screen.findByRole("dialog", { name: t.sourcePassword.title });
    await user.type(within(pwdDialog).getByLabelText(t.sourcePassword.label), "secret{Enter}");

    // Now replace dialog opens
    const dialog = await screen.findByRole("dialog", { name: t.replace.title });
    expect(dialog).toBeInTheDocument();
  });
});
