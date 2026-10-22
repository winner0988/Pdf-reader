import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ExportApi } from "@/features/export/api";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions, ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { AnnotationSummaryOutcome } from "@/ipc/generated/contract";

const t = strings.exportAnnotations;

function fakeExportApi(outcome: AnnotationSummaryOutcome | Error = { kind: "written", count: 3, truncated: false }) {
  return {
    exportPages: vi.fn<ExportApi["exportPages"]>(),
    cancel: vi.fn<ExportApi["cancel"]>(() => Promise.resolve()),
    exportAnnotationSummary: vi.fn<NonNullable<ExportApi["exportAnnotationSummary"]>>(() =>
      outcome instanceof Error ? Promise.reject(outcome) : Promise.resolve(outcome),
    ),
  } satisfies ExportApi;
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

const menuItem = () => screen.findByRole("menuitem", { name: new RegExp(`^${strings.menu.exportAnnotations}`) });

async function openSummary(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  await user.click(await menuItem());
  return screen.findByRole("dialog", { name: t.title });
}

const statusText = () => screen.getByRole("contentinfo").textContent ?? "";

describe("annotation summary (B3-18)", () => {
  it("asks for plain text starting with the file name by default, and says how many it lists", async () => {
    const api = fakeExportApi();
    const { user } = renderShell(api);
    const dialog = await openSummary(user);
    expect(within(dialog).getByRole("radio", { name: t.text })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: t.includeName })).toBeChecked();

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    // Only what kind of file: the main process makes it.
    expect(api.exportAnnotationSummary).toHaveBeenCalledWith({ doc: 5, format: "text", includeName: true });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(statusText()).toContain(t.done(3));
  });

  it("asks for Markdown without the file name, and says when the summary stopped before the end", async () => {
    const api = fakeExportApi({ kind: "written", count: 10000, truncated: true });
    const { user } = renderShell(api);
    const dialog = await openSummary(user);
    await user.click(within(dialog).getByRole("radio", { name: t.markdown }));
    await user.click(within(dialog).getByRole("checkbox", { name: t.includeName }));
    await user.click(within(dialog).getByRole("button", { name: t.start }));

    expect(api.exportAnnotationSummary).toHaveBeenCalledWith({ doc: 5, format: "markdown", includeName: false });
    await waitFor(() => expect(statusText()).toContain(t.truncated(10000)));
  });

  it("says when there is nothing to list, and stays open when the save dialog is closed", async () => {
    const empty = fakeExportApi({ kind: "empty" });
    const first = renderShell(empty);
    let dialog = await openSummary(first.user);
    await first.user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("status")).toHaveTextContent(t.empty);
    await first.user.click(within(dialog).getByRole("button", { name: t.cancel }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    // Opened again, it starts afresh.
    dialog = await openSummary(first.user);
    expect(within(dialog).queryByRole("status")).not.toBeInTheDocument();
    expect(within(dialog).getByRole("radio", { name: t.text })).toBeChecked();
  });

  it("keeps the dialog when the save dialog is closed, and says when writing fails", async () => {
    const cancelled = fakeExportApi({ kind: "cancelled" });
    const { user } = renderShell(cancelled);
    const dialog = await openSummary(user);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();
    expect(within(dialog).queryByRole("alert")).not.toBeInTheDocument();

    cancelled.exportAnnotationSummary.mockRejectedValueOnce(new Error("unwritable"));
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failed);
  });

  it("is not offered when the author forbids copying (MVP-19)", async () => {
    const { user } = renderShell(fakeExportApi(), {
      copy: false,
      print: true,
      printHighQuality: true,
      modify: true,
      assemble: true,
      annotate: true,
      fillForms: true,
    });
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    const blocked = await menuItem();
    expect(blocked).toHaveAttribute("data-disabled");
    expect(blocked).toHaveTextContent(strings.permissions.notAllowed);
  });

  it("is disabled without the summary in the export API", async () => {
    const { exportPages, cancel } = fakeExportApi();
    const { user } = renderShell({ exportPages, cancel });
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    const item = await menuItem();
    expect(item).toHaveAttribute("data-disabled");
    expect(item).not.toHaveTextContent(strings.permissions.notAllowed);
  });
});
