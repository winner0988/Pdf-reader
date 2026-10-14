// Compressing the pictures of a copy (B3-01): the dialog asks for the quality and the size, then for
// the copy, and says what came of it.
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { SavingApi } from "@/features/saving/api";
import { describe as describeResult, sizeText } from "@/features/saving/compress";
import { demoDocument } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { CompressResult } from "@/ipc/generated/contract";

const t = strings.compress;

const MIB = 1024 * 1024;
const result = (changes: Partial<CompressResult> = {}): CompressResult => ({
  before: 4 * MIB,
  after: 1 * MIB,
  images: 12,
  changed: 9,
  partial: false,
  ...changes,
});

function fakeSavingApi(compressImages: SavingApi["compressImages"]) {
  return {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false, redaction: null })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
    encryptCopy: vi.fn<SavingApi["encryptCopy"]>(() => Promise.resolve(true)),
    compressImages: vi.fn<SavingApi["compressImages"]>(compressImages),
  } satisfies SavingApi;
}

function renderShell(api?: SavingApi, encrypted = false) {
  const state: ShellState = { kind: "open", document: { ...demoDocument, doc: 5, encrypted } };
  const user = userEvent.setup();
  render(<ReaderShell state={state} onOpen={vi.fn()} loadingDelayMs={0} savingApi={api} />);
  return { user };
}

async function menuItem(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  return screen.findByRole("menuitem", { name: new RegExp(strings.menu.compressImages) });
}

async function openDialog(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await menuItem(user));
  return screen.findByRole("dialog", { name: t.title });
}

describe("what is said of a compressed copy (B3-01)", () => {
  it("gives sizes as a person says them", () => {
    expect(sizeText(0)).toBe(t.kilobytes(1));
    expect(sizeText(800)).toBe(t.kilobytes(1));
    expect(sizeText(1536)).toBe(t.kilobytes(2));
    expect(sizeText(MIB - 1)).toBe(t.kilobytes(1024));
    expect(sizeText(MIB)).toBe(t.megabytes("1.0"));
    expect(sizeText(4.26 * MIB)).toBe(t.megabytes("4.3"));
  });

  it("says how much smaller the copy is and how many pictures were made smaller", () => {
    expect(describeResult(result())).toBe(
      t.result.done(t.megabytes("4.0"), t.megabytes("1.0"), 75, 9, 12),
    );
    // Never less than one percent when something was saved.
    expect(describeResult(result({ before: 100_000, after: 99_999 }))).toBe(
      t.result.done(t.kilobytes(98), t.kilobytes(98), 1, 9, 12),
    );
  });

  it("says so when no picture changed, or there was none, and when the time was up", () => {
    expect(describeResult(result({ changed: 0 }))).toBe(t.result.unchanged(t.megabytes("1.0"), 12));
    // A copy that came out no smaller is not called compressed.
    expect(describeResult(result({ after: 5 * MIB }))).toBe(t.result.unchanged(t.megabytes("5.0"), 12));
    expect(describeResult(result({ images: 0, changed: 0 }))).toBe(t.result.none(t.megabytes("1.0")));
    expect(describeResult(result({ partial: true }))).toBe(
      t.result.done(t.megabytes("4.0"), t.megabytes("1.0"), 75, 9, 12) + t.result.partial,
    );
  });
});

describe("compressing the pictures of a copy (B3-01)", () => {
  it("asks for the quality and the size before anything is written, and starts with the usual ones", async () => {
    const api = fakeSavingApi(() => Promise.resolve(null));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    for (const text of [t.description, t.note, t.signatures]) expect(dialog).toHaveTextContent(text);
    expect(within(dialog).getByRole("radio", { name: t.quality.medium })).toBeChecked();
    expect(within(dialog).getByRole("radio", { name: t.size(2000) })).toBeChecked();
    expect(api.compressImages).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(api.compressImages).not.toHaveBeenCalled();
  });

  it("sends what was chosen, and says what came of it in the dialog", async () => {
    const api = fakeSavingApi(() => Promise.resolve(result()));
    const { user } = renderShell(api);
    const dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("radio", { name: t.quality.low }));
    await user.click(within(dialog).getByRole("radio", { name: t.sizeNone }));
    await user.click(within(dialog).getByRole("button", { name: t.start }));

    expect(api.compressImages).toHaveBeenCalledExactlyOnceWith({ doc: 5, quality: "low", maxSide: null });
    // It stays, with the result, until the user has read it.
    const status = await within(dialog).findByText(describeResult(result()));
    expect(status).toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: t.resultTitle })).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: t.close }));
    expect(screen.queryByRole("dialog")).toBeNull();
    // It is a copy: the document itself is not saved.
    expect(api.save).not.toHaveBeenCalled();
    expect(api.saveAs).not.toHaveBeenCalled();
  });

  it("asks for each size and quality it offers", async () => {
    const api = fakeSavingApi(() => Promise.resolve(result()));
    const { user } = renderShell(api);
    for (const [quality, side, maxSide] of [
      [t.quality.high, t.size(3000), 3000],
      [t.quality.medium, t.size(1500), 1500],
      [t.quality.low, t.size(1000), 1000],
    ] as const) {
      const dialog = await openDialog(user);
      await user.click(within(dialog).getByRole("radio", { name: quality }));
      await user.click(within(dialog).getByRole("radio", { name: side }));
      await user.click(within(dialog).getByRole("button", { name: t.start }));
      await within(dialog).findByText(describeResult(result()));
      const sent = api.compressImages.mock.calls.at(-1)![0];
      expect(sent.maxSide).toBe(maxSide);
      await user.click(within(dialog).getByRole("button", { name: t.close }));
    }
  });

  it("stays open for another try when the system's dialog is closed, and explains a failure", async () => {
    let answer: () => Promise<CompressResult | null> = () => Promise.resolve(null);
    const api = fakeSavingApi(() => answer());
    const { user } = renderShell(api);
    const dialog = await openDialog(user);

    await user.click(within(dialog).getByRole("button", { name: t.start }));
    await waitFor(() => expect(within(dialog).getByRole("button", { name: t.start })).toBeEnabled());
    expect(screen.getByRole("dialog", { name: t.title })).toBeInTheDocument();

    answer = () => Promise.reject({ code: "invalidRequest", message: "" });
    await user.click(within(dialog).getByRole("button", { name: t.start }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.failed);
    // And nothing was chosen away: another try with the same.
    expect(within(dialog).getByRole("radio", { name: t.quality.medium })).toBeChecked();
  });

  it("starts over each time it opens", async () => {
    const api = fakeSavingApi(() => Promise.resolve(result()));
    const { user } = renderShell(api);
    let dialog = await openDialog(user);
    await user.click(within(dialog).getByRole("radio", { name: t.quality.low }));
    await user.click(within(dialog).getByRole("button", { name: t.cancel }));
    dialog = await openDialog(user);
    expect(within(dialog).getByRole("radio", { name: t.quality.medium })).toBeChecked();
  });

  it("is not offered for an encrypted document, and says why", async () => {
    const api = fakeSavingApi(() => Promise.resolve(null));
    const { user } = renderShell(api, true);
    const item = await menuItem(user);
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(t.encrypted);
  });

  it("needs the main process (not demo data)", async () => {
    const { user } = renderShell(undefined);
    expect(await menuItem(user)).toHaveAttribute("aria-disabled", "true");
  });
});
