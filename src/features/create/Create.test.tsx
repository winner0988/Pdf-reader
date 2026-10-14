import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { CreateApi } from "@/features/create/api";
import { createFailure } from "@/features/create/messages";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { CreateResult } from "@/ipc/generated/contract";

const t = strings.create;

function fakeCreateApi(createPdf: CreateApi["createPdf"]) {
  return { createPdf: vi.fn<CreateApi["createPdf"]>(createPdf) } satisfies CreateApi;
}

const made: CreateResult = { pages: 3, kind: "docx" };

function renderShell(state: ShellState, api?: CreateApi) {
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} createApi={api} />);
  return { user };
}

const empty: ShellState = { kind: "empty" };
const open: ShellState = { kind: "open", document: { ...demoDocument, doc: 5 } };

async function openFromStart(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.empty.createButton }));
  return screen.findByRole("dialog", { name: t.title });
}

describe("making a PDF of files (B3-03)", () => {
  it("is offered on the start screen, and says what can be used and what is only an experiment", async () => {
    const api = fakeCreateApi(() => Promise.resolve(made));
    const { user } = renderShell(empty, api);
    const dialog = await openFromStart(user);

    for (const text of [t.description, ...t.kinds, t.limits, t.experimentalTitle, t.experimental, ...t.privacy]) {
      expect(dialog).toHaveTextContent(text);
    }
    // Nothing is asked of the main process before the user says so.
    expect(api.createPdf).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.createPdf).not.toHaveBeenCalled();
  });

  it("is not offered without the main process (demo data)", () => {
    renderShell(empty, undefined);
    expect(screen.queryByRole("button", { name: strings.empty.createButton })).toBeNull();
  });

  it("is in the menu of an open document too, and tells where the PDF is when it is made", async () => {
    const api = fakeCreateApi(() => Promise.resolve(made));
    const { user } = renderShell(open, api);
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: strings.menu.createPdf }));
    const dialog = await screen.findByRole("dialog", { name: t.title });

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(api.createPdf).toHaveBeenCalledExactlyOnceWith();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.done(3));
  });

  it("is a disabled item in the menu without the main process", async () => {
    const { user } = renderShell(open, undefined);
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    expect(await screen.findByRole("menuitem", { name: strings.menu.createPdf })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("cannot be closed or started again while it works", async () => {
    let finish: (result: CreateResult | null) => void = () => {};
    const api = fakeCreateApi(() => new Promise((resolve) => (finish = resolve)));
    const { user } = renderShell(empty, api);
    const dialog = await openFromStart(user);
    await user.click(within(dialog).getByRole("button", { name: t.start }));

    expect(await within(dialog).findByRole("status")).toHaveTextContent(t.running);
    expect(within(dialog).getByRole("button", { name: t.start })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: t.cancel })).toBeDisabled();
    await user.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();

    finish({ pages: 1, kind: "text" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("stays open when the system's dialog is closed, and says why it failed", async () => {
    let answer: () => Promise<CreateResult | null> = () => Promise.resolve(null);
    const api = fakeCreateApi(() => answer());
    const { user } = renderShell(empty, api);
    const dialog = await openFromStart(user);

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());
    expect(within(dialog).queryByRole("alert")).toBeNull();

    answer = () => Promise.reject({ code: "corrupted", message: "" });
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failures.corrupted!);

    // Another try starts without the last reason.
    answer = () => Promise.resolve(null);
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await waitFor(() => expect(within(dialog).queryByRole("alert")).toBeNull());

    // The reason is gone when the dialog is opened again.
    answer = () => Promise.reject({ code: "limitExceeded", message: "" });
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("alert")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    await user.click(screen.getByRole("button", { name: strings.empty.createButton }));
    expect(within(await screen.findByRole("dialog", { name: t.title })).queryByRole("alert")).toBeNull();
  });
});

describe("why a PDF could not be made", () => {
  it("has a text for the codes that mean something here, and the general one for the others", () => {
    expect(createFailure({ code: "invalidArgument", message: "" })).toBe(t.failures.invalidArgument);
    expect(createFailure({ code: "tooLarge", message: "" })).toBe(t.failures.tooLarge);
    expect(createFailure({ code: "workerTimeout", message: "" })).toBe(t.failures.workerTimeout);
    // The general text of a code the dialog does not say anything special about.
    expect(createFailure({ code: "diskFull", message: "" })).toBe(strings.error.messages.diskFull);
    expect(createFailure({ code: "readOnly", message: "" })).toBe(strings.error.messages.readOnly);
    // No code, a code without text, and something that is not an error at all.
    expect(createFailure({ code: "cancelled", message: "" })).toBe(t.failed);
    expect(createFailure(new Error("boom"))).toBe(strings.error.messages.internal);
    expect(createFailure(null)).toBe(strings.error.messages.internal);
  });
});
