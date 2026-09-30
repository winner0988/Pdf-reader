import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ExportApi } from "@/features/export/api";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions, ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
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

function renderShell(exportApi: ExportApi, permissions?: DocumentPermissions) {
  const state: ShellState = {
    kind: "open",
    document: { ...demoDocument, doc: 5, ...(permissions ? { permissions } : {}) },
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
    const { user } = renderShell(fake.api, { copy: false, print: true, printHighQuality: true, modify: true, assemble: true });
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    const item = await screen.findByRole("menuitem", { name: new RegExp(`^${strings.menu.export}`) });
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(strings.permissions.notAllowed);
  });
});
