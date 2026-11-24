import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { OcrApi } from "@/features/ocr/api";
import { SearchableCopyDialog, type SearchableCopyDialogProps } from "@/features/ocr/SearchableCopyDialog";
import { strings } from "@/i18n";
import type { OcrProgress, SearchableCopyArgs, SearchableCopyResult } from "@/ipc/generated/contract";

const t = strings.ocr.searchable;

function makeOcrApi(overrides: Partial<OcrApi> = {}): OcrApi {
  return {
    languages: vi.fn(() =>
      Promise.resolve({
        languages: [
          { code: "chi_tra", bundled: true, bytes: 2_000_000n },
          { code: "eng", bundled: true, bytes: 1_000_000n },
        ],
        automatic: "chi_tra",
      }),
    ),
    importLanguage: vi.fn(),
    removeLanguage: vi.fn(),
    start: vi.fn(() => Promise.resolve()),
    stop: vi.fn(() => Promise.resolve()),
    focus: vi.fn(() => Promise.resolve()),
    ...overrides,
  };
}

function renderDialog(overrides: Partial<SearchableCopyDialogProps> = {}) {
  const user = userEvent.setup();
  const props: SearchableCopyDialogProps = {
    open: true,
    onOpenChange: vi.fn(),
    doc: 1,
    pageCount: 4,
    ocrProgress: null,
    ocrApi: makeOcrApi(),
    onSaveSearchable: vi.fn(() =>
      Promise.resolve({
        addedPages: 2,
        skipped: [],
      }),
    ),
    ...overrides,
  };
  const view = render(<SearchableCopyDialog {...props} />);
  return { ...view, user, props };
}

describe("SearchableCopyDialog", () => {
  it("shows OCR language and page count, runs OCR, saves on completion, and lists skipped pages", async () => {
    const result: SearchableCopyResult = {
      addedPages: 1,
      skipped: [
        { pageIndex: 0, reason: "notScan" },
        { pageIndex: 2, reason: "noText" },
        { pageIndex: 3, reason: "timedOut" },
        { pageIndex: 4, reason: "failed" },
      ],
    };
    const onSaveSearchable = vi.fn(() => Promise.resolve(result));
    const ocrApi = makeOcrApi();
    const initialProgress: OcrProgress = {
      doc: 1,
      run: "idle",
      pages: 5,
      checked: 0,
      scans: 0,
      recognised: 0,
      failed: 0,
    };

    const { user, rerender, props } = renderDialog({
      pageCount: 5,
      ocrProgress: initialProgress,
      ocrApi,
      onSaveSearchable,
    });

    await screen.findByText(strings.ocr.settings.automatic("繁體中文（chi_tra）"));
    expect(screen.getByText(t.pagesSummary(5))).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: t.start }));
    expect(ocrApi.start).toHaveBeenCalledWith(1);
    expect(onSaveSearchable).not.toHaveBeenCalled();

    // Progress updates while running.
    const runningProgress: OcrProgress = {
      doc: 1,
      run: "running",
      pages: 5,
      checked: 3,
      scans: 2,
      recognised: 1,
      failed: 0,
    };
    rerender(<SearchableCopyDialog {...props} ocrProgress={runningProgress} />);
    expect(screen.getByRole("status")).toHaveTextContent(t.recognisingScans(1, 2, 3, 5));

    // OCR completes -> automatically saves copy.
    const doneProgress: OcrProgress = {
      doc: 1,
      run: "done",
      pages: 5,
      checked: 5,
      scans: 4,
      recognised: 1,
      failed: 2,
    };
    rerender(<SearchableCopyDialog {...props} ocrProgress={doneProgress} />);

    await waitFor(() => expect(onSaveSearchable).toHaveBeenCalledWith({ doc: 1 }));
    await screen.findByText(t.doneTitle);
    expect(screen.getByText(t.doneAdded(1))).toBeInTheDocument();
    expect(screen.getByText(t.skippedPage(1, t.skipReasons.notScan))).toBeInTheDocument();
    expect(screen.getByText(t.skippedPage(3, t.skipReasons.noText))).toBeInTheDocument();
    expect(screen.getByText(t.skippedPage(4, t.skipReasons.timedOut))).toBeInTheDocument();
    expect(screen.getByText(t.skippedPage(5, t.skipReasons.failed))).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: t.doneButton }));
    expect(props.onOpenChange).toHaveBeenCalledWith(false);
  });

  it("handles a single-tick idle to done transition", async () => {
    const onSaveSearchable = vi.fn(() => Promise.resolve({ addedPages: 0, skipped: [] }));
    const ocrApi = makeOcrApi();
    const { user, rerender, props } = renderDialog({
      ocrProgress: { doc: 1, run: "idle", pages: 4, checked: 0, scans: 0, recognised: 0, failed: 0 },
      ocrApi,
      onSaveSearchable,
    });

    await user.click(screen.getByRole("button", { name: t.start }));
    rerender(
      <SearchableCopyDialog
        {...props}
        ocrProgress={{ doc: 1, run: "done", pages: 4, checked: 4, scans: 0, recognised: 0, failed: 0 }}
      />,
    );

    await waitFor(() => expect(onSaveSearchable).toHaveBeenCalledWith({ doc: 1 }));
    await screen.findByText(t.doneNoneAdded);
  });

  it("saves directly without restarting OCR when recognition is already done", async () => {
    const ocrApi = makeOcrApi();
    const onSaveSearchable = vi.fn(() => Promise.resolve({ addedPages: 2, skipped: [] }));
    const { user } = renderDialog({
      ocrProgress: { doc: 1, run: "done", pages: 4, checked: 4, scans: 2, recognised: 2, failed: 0 },
      ocrApi,
      onSaveSearchable,
    });

    expect(screen.getByText(t.pagesDoneSummary(4))).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: t.startSave }));

    expect(ocrApi.start).not.toHaveBeenCalled();
    await waitFor(() => expect(onSaveSearchable).toHaveBeenCalledWith({ doc: 1 }));
    await screen.findByText(t.doneAdded(2));
  });

  it("stops OCR and does not save when cancelled during recognition", async () => {
    const ocrApi = makeOcrApi();
    const onSaveSearchable = vi.fn();
    const { user, rerender, props } = renderDialog({
      ocrProgress: { doc: 1, run: "idle", pages: 4, checked: 0, scans: 0, recognised: 0, failed: 0 },
      ocrApi,
      onSaveSearchable,
    });

    await user.click(screen.getByRole("button", { name: t.start }));
    await user.click(screen.getByRole("button", { name: t.cancel }));
    expect(ocrApi.stop).toHaveBeenCalledWith(1);

    // Even if a late done event arrives afterwards, saving must not start.
    rerender(
      <SearchableCopyDialog
        {...props}
        ocrProgress={{ doc: 1, run: "done", pages: 4, checked: 4, scans: 1, recognised: 1, failed: 0 }}
      />,
    );
    expect(onSaveSearchable).not.toHaveBeenCalled();
  });

  it("explains why encrypted documents cannot be saved and notes in-app search still works", () => {
    const ocrApi = makeOcrApi();
    renderDialog({ encrypted: true, ocrApi });

    expect(screen.getByText(t.encryptedReason)).toBeInTheDocument();
    expect(screen.getByText(t.stillWorksInApp)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t.ok })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: t.start })).not.toBeInTheDocument();
    expect(ocrApi.languages).not.toHaveBeenCalled();
  });

  it("explains why signed documents cannot be saved and notes in-app search still works", () => {
    const ocrApi = makeOcrApi();
    renderDialog({ signed: true, ocrApi });

    expect(screen.getByText(t.signedReason)).toBeInTheDocument();
    expect(screen.getByText(t.stillWorksInApp)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: t.ok })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: t.start })).not.toBeInTheDocument();
  });

  it("returns to idle when the user cancels the system save dialog", async () => {
    const onSaveSearchable = vi.fn(() => Promise.resolve(null));
    const { user } = renderDialog({
      ocrProgress: { doc: 1, run: "done", pages: 4, checked: 4, scans: 1, recognised: 1, failed: 0 },
      onSaveSearchable,
    });

    await user.click(screen.getByRole("button", { name: t.startSave }));
    await waitFor(() => expect(onSaveSearchable).toHaveBeenCalledWith({ doc: 1 }));
    expect(screen.getByRole("button", { name: t.startSave })).toBeEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("surfaces errors from OCR or saving", async () => {
    const onSaveSearchable = vi.fn(() => Promise.reject({ code: "signed", message: "signed" }));
    const { user } = renderDialog({
      ocrProgress: { doc: 1, run: "done", pages: 4, checked: 4, scans: 1, recognised: 1, failed: 0 },
      onSaveSearchable,
    });

    await user.click(screen.getByRole("button", { name: t.startSave }));
    expect(await screen.findByRole("alert")).toHaveTextContent(t.signedReason);
  });

  it("does not re-invoke saving or discard the result when the parent re-renders with a new callback", async () => {
    let resolveSave!: (value: SearchableCopyResult | null) => void;
    const invokeSave = vi.fn<(args: SearchableCopyArgs) => Promise<SearchableCopyResult | null>>(
      () =>
        new Promise<SearchableCopyResult | null>((resolve) => {
          resolveSave = resolve;
        }),
    );
    const ocrApi = makeOcrApi();
    const initialProgress: OcrProgress = {
      doc: 1,
      run: "idle",
      pages: 1,
      checked: 0,
      scans: 0,
      recognised: 0,
      failed: 0,
    };
    const { user, rerender, props } = renderDialog({
      pageCount: 1,
      ocrProgress: initialProgress,
      ocrApi,
      onSaveSearchable: (args) => invokeSave(args),
    });

    await user.click(screen.getByRole("button", { name: t.start }));

    const doneProgress: OcrProgress = {
      doc: 1,
      run: "done",
      pages: 1,
      checked: 1,
      scans: 1,
      recognised: 1,
      failed: 0,
    };
    rerender(
      <SearchableCopyDialog
        {...props}
        ocrProgress={doneProgress}
        onSaveSearchable={(args) => invokeSave(args)}
      />,
    );
    await waitFor(() => expect(invokeSave).toHaveBeenCalledTimes(1));

    // Simulate ReaderShell re-rendering (e.g. OCR note/page text update) with a fresh inline callback.
    rerender(
      <SearchableCopyDialog
        {...props}
        ocrProgress={{ ...doneProgress }}
        onSaveSearchable={(args) => invokeSave(args)}
      />,
    );
    rerender(
      <SearchableCopyDialog
        {...props}
        ocrProgress={{ ...doneProgress }}
        onSaveSearchable={(args) => invokeSave(args)}
      />,
    );

    expect(invokeSave).toHaveBeenCalledTimes(1);

    resolveSave({ addedPages: 1, skipped: [] });
    await screen.findByText(t.doneTitle);
    expect(screen.getByText(t.doneAdded(1))).toBeInTheDocument();
  });
});
