import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ExportApi } from "@/features/export/api";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions, ShellDocument, ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";

const t = strings.export;

/** An export that finishes, fails or reports progress when the test says so. */
function fakeExportApi() {
  let finish: (exported: boolean) => void = () => {};
  let fail: (error: unknown) => void = () => {};
  let progress: (done: number, total: number) => void = () => {};
  const api = {
    exportPages: vi.fn<ExportApi["exportPages"]>((_doc, _pages, _format, onProgress) => {
      progress = onProgress;
      return {
        request: 42,
        done: new Promise<boolean>((resolve, reject) => {
          finish = resolve;
          fail = reject;
        }),
      };
    }),
    cancel: vi.fn<ExportApi["cancel"]>(() => Promise.resolve()),
  } satisfies ExportApi;
  return {
    api,
    finish: (exported: boolean) => act(() => finish(exported)),
    fail: (error: unknown) => act(() => fail(error)),
    progress: (done: number, total: number) => act(() => progress(done, total)),
  };
}

function renderShell(exportApi: ExportApi, permissions?: DocumentPermissions, document: Partial<ShellDocument> = {}) {
  const state: ShellState = {
    kind: "open",
    document: { ...demoDocument, doc: 5, ...(permissions ? { permissions } : {}), ...document },
  };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} exportApi={exportApi} />);
  return { user };
}

async function openExport(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  await user.click(await screen.findByRole("menuitem", { name: new RegExp(`^${strings.menu.export}`) }));
  return screen.findByRole("dialog", { name: t.title });
}

const statusText = () => screen.getByRole("contentinfo").textContent ?? "";

describe("export (B2-04)", () => {
  it("exports the text of every page by default, and says how many", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api);
    const dialog = await openExport(user);

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    const pages = demoDocument.pages.map((_, index) => index);
    expect(fake.api.exportPages).toHaveBeenCalledWith(5, pages, { kind: "text" }, expect.any(Function));

    await fake.finish(true);
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(statusText()).toContain(t.done(pages.length));
  });

  it("exports chosen pages as PNG at the chosen resolution, shows progress and can stop", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api);
    const dialog = await openExport(user);

    await user.click(within(dialog).getByRole("radio", { name: t.png }));
    await user.selectOptions(within(dialog).getByRole("combobox", { name: t.resolution }), "300");
    await user.type(within(dialog).getByRole("textbox", { name: strings.print.pages }), "2-3");
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(fake.api.exportPages).toHaveBeenCalledWith(5, [1, 2], { kind: "png", dpi: 300 }, expect.any(Function));

    await fake.progress(1, 2);
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.progress(1, 2));
    await user.click(within(dialog).getByRole("button", { name: t.stop }));
    expect(fake.api.cancel).toHaveBeenCalledWith(42);

    await fake.fail({ code: "cancelled", message: "cancelled" });
    await waitFor(() => expect(statusText()).toContain(t.stopped(1)));
  });

  it("exports pages as JPG at the resolution the images share (#111)", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api);
    const dialog = await openExport(user);
    const resolution = within(dialog).getByRole("combobox", { name: t.resolution });
    // The resolution is for images only.
    expect(resolution).toBeDisabled();

    await user.click(within(dialog).getByRole("radio", { name: t.jpg }));
    expect(resolution).toBeEnabled();
    await user.selectOptions(resolution, "72");
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    const pages = demoDocument.pages.map((_, index) => index);
    expect(fake.api.exportPages).toHaveBeenCalledWith(5, pages, { kind: "jpg", dpi: 72 }, expect.any(Function));
  });

  it("saves the pages chosen as one PDF file (B2-06)", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api);
    const dialog = await openExport(user);

    await user.click(within(dialog).getByRole("radio", { name: t.pdf }));
    // Not images: no resolution.
    expect(within(dialog).getByRole("combobox", { name: t.resolution })).toBeDisabled();
    await user.type(within(dialog).getByRole("textbox", { name: strings.print.pages }), "2-4, 7");
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(fake.api.exportPages).toHaveBeenCalledWith(5, [1, 2, 3, 6], { kind: "pdf" }, expect.any(Function));

    await fake.finish(true);
    await waitFor(() => expect(statusText()).toContain(t.done(4)));
  });

  it("splits the document into files of so many pages, and says how many files (B2-06)", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api);
    const dialog = await openExport(user);

    const perFile = within(dialog).getByRole("textbox", { name: t.perFile });
    await user.clear(perFile);
    await user.type(perFile, "5");
    // Typing in the box chooses what it is for.
    expect(within(dialog).getByRole("radio", { name: t.pdfEvery })).toBeChecked();
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    const pages = demoDocument.pages.map((_, index) => index);
    expect(fake.api.exportPages).toHaveBeenCalledWith(5, pages, { kind: "pdfEvery", count: 5 }, expect.any(Function));

    await fake.finish(true);
    // Twelve pages, five to a file: three files.
    await waitFor(() => expect(statusText()).toContain(t.splitDone(3, pages.length)));
  });

  it("asks for a number of pages for each file, and not for more files than it makes", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api, undefined, {
      pages: Array.from({ length: 3000 }, () => ({ widthPt: 612, heightPt: 792 })),
    });
    const dialog = await openExport(user);
    const perFile = within(dialog).getByRole("textbox", { name: t.perFile });
    const start = () => user.click(within(dialog).getByRole("button", { name: t.start }));

    await user.click(within(dialog).getByRole("radio", { name: t.pdfEvery }));
    for (const wrong of ["", "0", "abc", "2.5", "-3"]) {
      await user.clear(perFile);
      if (wrong) await user.type(perFile, wrong);
      await start();
      expect(within(dialog).getByRole("alert")).toHaveTextContent(t.perFileInvalid);
    }
    // 3000 pages a file each: more files than one split makes.
    await user.clear(perFile);
    await user.type(perFile, "1");
    await start();
    expect(within(dialog).getByRole("alert")).toHaveTextContent(t.tooManyFiles(1000));
    expect(fake.api.exportPages).not.toHaveBeenCalled();
    // Three to a file is exactly a thousand files, and all of its 3000 pages (a PDF takes more
    // pages than text or images do).
    await user.clear(perFile);
    await user.type(perFile, "3");
    await start();
    expect(fake.api.exportPages).toHaveBeenCalledWith(
      5,
      expect.arrayContaining([0, 2999]),
      { kind: "pdfEvery", count: 3 },
      expect.any(Function),
    );
  });

  it("does not offer PDF files for an encrypted document, and says why (B2-06)", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api, undefined, { encrypted: true });
    const dialog = await openExport(user);
    expect(within(dialog).getByRole("radio", { name: t.pdf })).toBeDisabled();
    expect(within(dialog).getByRole("radio", { name: t.pdfEvery })).toBeDisabled();
    expect(within(dialog).getByText(t.encryptedNote)).toBeInTheDocument();
    // The other formats are as they were.
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(fake.api.exportPages).toHaveBeenCalledWith(
      5,
      expect.any(Array),
      { kind: "text" },
      expect.any(Function),
    );
  });

  it("opens with the pages chosen in the thumbnails, as a PDF file (B2-06)", async () => {
    const fake = fakeExportApi();
    const editingApi = {
      applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
      undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
      redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
      recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
      discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
    } satisfies EditingApi;
    const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5 } };
    render(
      <ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} exportApi={fake.api} editingApi={editingApi} />,
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));
    const thumb = (number: number) => screen.getByRole("option", { name: strings.canvas.page(number) });
    await user.click(thumb(2));
    await user.keyboard("{Shift>}");
    await user.click(thumb(4));
    await user.keyboard("{/Shift}{Control>}");
    await user.click(thumb(6));
    await user.keyboard("{/Control}");
    fireEvent.contextMenu(thumb(6));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(`^${strings.pages.saveSelected}`) }));

    const dialog = await screen.findByRole("dialog", { name: t.title });
    expect(within(dialog).getByRole("radio", { name: t.pdf })).toBeChecked();
    expect(within(dialog).getByRole("textbox", { name: strings.print.pages })).toHaveValue("2-4, 6");
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(fake.api.exportPages).toHaveBeenCalledWith(5, [1, 2, 3, 5], { kind: "pdf" }, expect.any(Function));
    await fake.finish(false);

    // Opened from the menu, it starts over: every page.
    await user.keyboard("{Escape}");
    const again = await openExport(user);
    expect(within(again).getByRole("textbox", { name: strings.print.pages })).toHaveValue("");
    expect(within(again).getByRole("radio", { name: strings.print.all(demoDocument.pages.length) })).toBeChecked();
  });

  it("stays open when the user closes the system's dialog, and explains failures", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api);
    const dialog = await openExport(user);

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await fake.finish(false);
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await fake.fail({ code: "unreadable", message: "" });
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failed);
  });

  it("explains a page range that is not in the document", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api);
    const dialog = await openExport(user);

    await user.type(within(dialog).getByRole("textbox", { name: strings.print.pages }), "99");
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(within(dialog).getByRole("alert")).toHaveTextContent(strings.print.invalid(demoDocument.pages.length));
    expect(fake.api.exportPages).not.toHaveBeenCalled();
  });

  it("is not offered when the author forbids copying (MVP-19)", async () => {
    const fake = fakeExportApi();
    const { user } = renderShell(fake.api, { copy: false, print: true, printHighQuality: true, modify: true, assemble: true, annotate: true, fillForms: true });
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    const item = await screen.findByRole("menuitem", { name: new RegExp(`^${strings.menu.export}`) });
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(strings.permissions.notAllowed);
  });
});
