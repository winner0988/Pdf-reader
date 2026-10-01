import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import type { ShellDocument, ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";

const t = strings.recovery;

function fakeEditingApi() {
  return {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
}

function renderShell(document: Partial<ShellDocument>) {
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

const banner = () => screen.queryByRole("region", { name: t.label });

describe("changes an earlier run left (B2-13)", () => {
  it("are offered, and restored", async () => {
    const { api, rerender, user } = renderShell({ recovery: "available" });
    const region = banner()!;
    expect(region).toHaveTextContent(t.available);
    await user.click(within(region).getByRole("button", { name: t.restore }));
    expect(api.recover).toHaveBeenCalledWith(5);
    // The tab's new state comes from the main process: the edits are made, nothing is offered.
    rerender({ doc: 6, recovery: "none", unsaved: true, canUndo: true });
    expect(banner()).toBeNull();
  });

  it("can be discarded", async () => {
    const { api, user } = renderShell({ recovery: "available" });
    await user.click(within(banner()!).getByRole("button", { name: t.discard }));
    expect(api.discardRecovered).toHaveBeenCalledWith(5);
    expect(api.recover).not.toHaveBeenCalled();
  });

  it("cannot be restored once the file changed, only discarded", async () => {
    const { api, user } = renderShell({ recovery: "stale" });
    const region = banner()!;
    expect(region).toHaveTextContent(t.stale);
    expect(within(region).queryByRole("button", { name: t.restore })).toBeNull();
    await user.click(within(region).getByRole("button", { name: t.discard }));
    expect(api.discardRecovered).toHaveBeenCalledWith(5);
  });

  it("can be left for later, without an answer", async () => {
    const { api, user } = renderShell({ recovery: "available" });
    await user.click(within(banner()!).getByRole("button", { name: t.later }));
    expect(banner()).toBeNull();
    expect(api.recover).not.toHaveBeenCalled();
    expect(api.discardRecovered).not.toHaveBeenCalled();
  });

  it("says why restoring is refused while the document has edits of its own", async () => {
    const { api, user } = renderShell({ recovery: "available", unsaved: true, canUndo: true });
    api.recover.mockRejectedValue({ code: "invalidArgument", message: "" });
    await user.click(within(banner()!).getByRole("button", { name: t.restore }));
    await waitFor(() => expect(screen.getByRole("contentinfo")).toHaveTextContent(t.ownChanges));
    // Still offered.
    expect(banner()).not.toBeNull();
  });

  it("is not offered for a document without any", () => {
    renderShell({ recovery: "none" });
    expect(banner()).toBeNull();
  });
});
