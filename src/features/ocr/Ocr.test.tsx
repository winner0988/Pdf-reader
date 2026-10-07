import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { OcrApi } from "@/features/ocr/api";
import { FOCUS_DELAY_MS } from "@/features/ocr/useOcrView";
import type { OcrTab } from "@/features/ocr/model";
import { demoDocument } from "@/features/shell/demo";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { TextApi } from "@/features/text/source";
import { strings } from "@/i18n/zh-TW";
import type { SearchApi } from "@/features/search/useSearch";
import type { OcrProgress, PageText, SearchEvent } from "@/ipc/generated/contract";

const t = strings.ocr;

const progress = (patch: Partial<OcrProgress> = {}): OcrProgress => ({
  doc: 5 as OcrProgress["doc"],
  run: "running",
  pages: 12,
  checked: 6,
  scans: 4,
  recognised: 1,
  failed: 0,
  ...patch,
});

const tab = (patch: Partial<OcrProgress> = {}, versions: [number, number][] = []): OcrTab => ({
  progress: progress(patch),
  doc: 5 as OcrProgress["doc"],
  versions: new Map(versions),
});

function fakeApi() {
  return {
    languages: vi.fn<OcrApi["languages"]>(() => Promise.resolve({ languages: [], automatic: null })),
    importLanguage: vi.fn<OcrApi["importLanguage"]>(() => Promise.resolve({ kind: "cancelled" })),
    removeLanguage: vi.fn<OcrApi["removeLanguage"]>(() => Promise.resolve({ languages: [], automatic: null })),
    start: vi.fn<OcrApi["start"]>(() => Promise.resolve()),
    stop: vi.fn<OcrApi["stop"]>(() => Promise.resolve()),
    focus: vi.fn<OcrApi["focus"]>(() => Promise.resolve()),
  } satisfies OcrApi;
}

const text = (recognised: boolean): PageText => ({ lines: [], truncated: false, recognised });

function setup(ocr: OcrTab | undefined, textApi?: TextApi, searchApi?: SearchApi) {
  const api = fakeApi();
  // The same document object for every render: another one is another file to the shell.
  const state = { kind: "open", document: { ...demoDocument, doc: 5 } } as const;
  const ui = (current: OcrTab | undefined) => (
    <ReaderShell
      state={state}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      ocrApi={api}
      ocr={current}
      textApi={textApi}
      searchApi={searchApi}
    />
  );
  const view = render(ui(ocr));
  return { api, rerender: (next: OcrTab | undefined) => view.rerender(ui(next)) };
}

describe("recognising the text of scanned pages (B2-10)", () => {
  it("shows how many of the scanned pages found are read, with a way to stop", async () => {
    const { api } = setup(tab({ recognised: 1, failed: 1, scans: 4 }));
    expect(screen.getByText(t.running(2, 4))).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: t.stopLabel }));
    expect(api.stop).toHaveBeenCalledWith(5);
  });

  it("says nothing while no scanned page has been found", () => {
    setup(tab({ scans: 0, recognised: 0, checked: 3 }));
    expect(screen.queryByRole("button", { name: t.stopLabel })).toBeNull();
    expect(screen.queryByText(/辨識文字：/)).toBeNull();
  });

  it("starts from the menu, pointing at the page in view, and cannot be started twice", async () => {
    const { api, rerender } = setup(tab({ run: "done" }));
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: t.menu }));
    expect(api.start).toHaveBeenCalledWith(5);
    expect(api.focus).toHaveBeenCalledWith(5, 0);

    rerender(tab({ run: "running" }));
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    expect(await screen.findByRole("menuitem", { name: t.menuRunning })).toHaveAttribute("aria-disabled", "true");
  });

  it("tells the main process which page is in view, once the page has stayed there a moment", async () => {
    const { api } = setup(tab());
    await waitFor(() => expect(api.focus).toHaveBeenCalledWith(5, 0), { timeout: FOCUS_DELAY_MS * 5 });
  });

  it("says how it ended: read, stopped, no language, failed", async () => {
    const { rerender } = setup(tab({ run: "running" }));
    const footer = () => within(screen.getByRole("contentinfo"));
    rerender(tab({ run: "done", recognised: 3, failed: 1, scans: 4 }));
    expect(await footer().findByText(t.done(3, 1))).toBeInTheDocument();

    rerender(tab({ run: "running" }));
    rerender(tab({ run: "stopped" }));
    expect(await footer().findByText(t.stopped)).toBeInTheDocument();

    rerender(tab({ run: "running" }));
    rerender(tab({ run: "noLanguage" }));
    expect(await footer().findByText(t.noLanguage)).toBeInTheDocument();

    rerender(tab({ run: "running" }));
    rerender(tab({ run: "failed" }));
    expect(await footer().findByText(t.failed)).toBeInTheDocument();
  });

  it("says nothing when a document without scans is done, unless the user asked", async () => {
    const { rerender } = setup(tab({ run: "running", scans: 0, recognised: 0 }));
    rerender(tab({ run: "done", scans: 0, recognised: 0 }));
    expect(screen.queryByText(t.none)).toBeNull();

    const user = userEvent.setup();
    rerender(tab({ run: "idle", scans: 0, recognised: 0 }));
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: t.menu }));
    rerender(tab({ run: "running", scans: 0, recognised: 0 }));
    rerender(tab({ run: "done", scans: 0, recognised: 0 }));
    expect(await screen.findByText(t.none)).toBeInTheDocument();
  });

  it("notes that the text of the page shown was recognised, and asks for it again when it is new", async () => {
    const getPageText = vi.fn((_doc: number, page: number) => Promise.resolve(text(page === 0)));
    const { rerender } = setup(tab({}, []), { getPageText });
    expect(await screen.findByTestId("ocr-note")).toHaveTextContent(t.pageNote);
    const asked = getPageText.mock.calls.filter(([, page]) => page === 0).length;

    // Page 0's text was recognised again (a rotated page, say): it is asked for again.
    rerender(tab({}, [[0, 1]]));
    await waitFor(() => expect(getPageText.mock.calls.filter(([, page]) => page === 0).length).toBe(asked + 1));
  });

  it("searches again when more scanned pages have text, once it has gone quiet, but only a search that was done", async () => {
    const searchApi = {
      search: vi.fn((_args, onEvent: (event: SearchEvent) => void) => {
        onEvent({ kind: "done", totalHits: 0, truncated: false, noTextLayer: true });
        return Promise.resolve();
      }),
      cancel: vi.fn(() => Promise.resolve()),
    } satisfies SearchApi;
    const { rerender } = setup(tab(), undefined, searchApi);
    const user = userEvent.setup();

    // The search bar is closed: nothing is searched for the new text.
    rerender(tab({}, [[0, 1]]));
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1300));
    });
    expect(searchApi.search).not.toHaveBeenCalled();

    await user.keyboard("{Control>}f{/Control}needle{Enter}");
    await waitFor(() => expect(searchApi.search).toHaveBeenCalledTimes(1));
    rerender(tab({}, [[0, 2]]));
    await waitFor(() => expect(searchApi.search).toHaveBeenCalledTimes(2), { timeout: 3000 });
    expect(searchApi.search).toHaveBeenLastCalledWith(
      expect.objectContaining({ doc: 5, query: "needle" }),
      expect.any(Function),
    );
  });

  it("does not note a page whose text is its own", async () => {
    const getPageText = vi.fn(() => Promise.resolve(text(false)));
    setup(tab(), { getPageText });
    await waitFor(() => expect(getPageText).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.queryByTestId("ocr-note")).toBeNull();
  });
});
