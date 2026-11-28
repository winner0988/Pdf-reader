import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { OcrApi } from "@/features/ocr/api";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { SignatureView } from "@/features/signatures/useSignatures";
import type { EditingApi } from "@/features/thumbnails/api";
import type { PageRenderer } from "@/features/viewer/renderer";
import type { WindowApi } from "@/features/window/api";
import { strings } from "@/i18n";
import {
  LIMITS,
  type DocumentId,
  type Edit,
  type RecognisedWord,
  type RecognisedWordsPage,
} from "@/ipc/generated/contract";
import { expectAccessible } from "@/test/axe";

const word = (id: number, text: string, x0 = 72 + id * 80, y0 = 100): RecognisedWord => ({
  id,
  text,
  rect: { x0, y0, x1: x0 + 60, y1: y0 + 18 },
  quad: {
    ul: { x: x0, y: y0 },
    ur: { x: x0 + 60, y: y0 },
    ll: { x: x0, y: y0 + 18 },
    lr: { x: x0 + 60, y: y0 + 18 },
  },
});

const WORDS_PAGE: RecognisedWord[] = [word(0, "Hel1o", 72, 100), word(1, "W0rld", 160, 100)];

const SIGNED_VIEW: SignatureView = {
  status: "ready",
  report: {
    signatures: [
      {
        status: "valid",
        signerTrusted: true,
        reason: null,
        fieldName: "Sig1",
        signer: "Alice",
        claimedTime: null,
        certification: null,
      },
    ],
    truncated: false,
  },
};

function makeOcrApi(
  getWords: (doc: DocumentId, pageIndex: number) => Promise<RecognisedWordsPage> = (_doc, pageIndex) =>
    Promise.resolve(pageIndex === 0 ? { status: "usable", words: WORDS_PAGE } : { status: "unusable", reason: "noTextLayer" }),
): OcrApi & { getRecognisedWords: ReturnType<typeof vi.fn> } {
  const getRecognisedWords = vi.fn(getWords);
  return {
    languages: vi.fn<OcrApi["languages"]>(() => Promise.resolve({ languages: [], automatic: null })),
    importLanguage: vi.fn<OcrApi["importLanguage"]>(() => Promise.resolve({ kind: "cancelled" })),
    removeLanguage: vi.fn<OcrApi["removeLanguage"]>(() => Promise.resolve({ languages: [], automatic: null })),
    start: vi.fn<OcrApi["start"]>(() => Promise.resolve()),
    stop: vi.fn<OcrApi["stop"]>(() => Promise.resolve()),
    focus: vi.fn<OcrApi["focus"]>(() => Promise.resolve()),
    getRecognisedWords,
  };
}

async function setup(options: {
  permissions?: DocumentPermissions;
  signatures?: SignatureView;
  getWords?: (doc: DocumentId, pageIndex: number) => Promise<RecognisedWordsPage>;
  applyEdit?: EditingApi["applyEdit"];
  windowApi?: WindowApi;
  renderer?: PageRenderer;
} = {}) {
  const editing = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(options.applyEdit ?? (() => Promise.resolve())),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;

  const ocrApi = makeOcrApi(options.getWords);
  const renderer: PageRenderer = options.renderer ?? {
    render: vi.fn(() => ({
      result: Promise.resolve({ width: 10, height: 10, pixels: new Uint8ClampedArray(400) }),
      cancel: vi.fn(),
    })),
  };

  const ui = (docId = 5) => (
    <ReaderShell
      state={{
        kind: "open",
        document: {
          ...demoDocument,
          doc: docId,
          session: 1,
          permissions: options.permissions ?? ALL_PERMISSIONS,
        },
      }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      editingApi={editing}
      ocrApi={ocrApi}
      signatures={options.signatures}
      windowApi={options.windowApi}
      renderer={renderer}
    />
  );

  const view = render(ui(5));
  const canvas = screen.getByRole("main");
  Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
  Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 800 });
  act(() => canvas.dispatchEvent(new Event("scroll")));
  await act(async () => {});

  const user = userEvent.setup({ pointerEventsCheck: PointerEventsCheckLevel.Never });
  return { ...view, rerenderDoc: (docId: number) => view.rerender(ui(docId)), editing, ocrApi, renderer, user };
}

const lastEdit = (editing: { applyEdit: ReturnType<typeof vi.fn> }): Edit =>
  editing.applyEdit.mock.calls.at(-1)![1] as Edit;

describe("correcting recognised text in the invisible text layer (#749, B7-12)", () => {
  it("opens from the More menu and Command Palette, lists all 3 Tr words without marking uncertainty, and exits on Esc", async () => {
    const windowApi = { setFullscreen: vi.fn((on: boolean) => Promise.resolve(on)) } satisfies WindowApi;
    const { ocrApi, user } = await setup({ windowApi });

    // Enter fullscreen first to ensure Esc closes correction mode before exiting fullscreen.
    await act(async () => {
      fireEvent.keyDown(window, { key: "F11" });
    });
    windowApi.setFullscreen.mockClear();

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    const item = await screen.findByRole("menuitem", { name: "修正辨識的文字" });
    await user.click(item);

    await waitFor(() => expect(ocrApi.getRecognisedWords).toHaveBeenCalledWith(5, 0));
    const firstWordBtn = await screen.findByRole("button", { name: /Hel1o/ });
    const secondWordBtn = screen.getByRole("button", { name: /W0rld/ });
    expect(firstWordBtn).toBeInTheDocument();
    expect(secondWordBtn).toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).toHaveTextContent(/點一下文字|Esc/);

    await expectAccessible(document.body);

    // Esc exits correction mode first without exiting fullscreen.
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("button", { name: /Hel1o/ })).toBeNull());
    expect(windowApi.setFullscreen).not.toHaveBeenCalled();

    // Can also launch from Ctrl+K Command Palette.
    await user.keyboard("{Control>}k{/Control}");
    const paletteInput = await screen.findByPlaceholderText(strings.commands.palettePlaceholder);
    await user.type(paletteInput, "修正辨識的文字");
    const cmdOption = await screen.findByRole("option", { name: /修正辨識的文字/ });
    await user.click(cmdOption);
    await screen.findByRole("button", { name: /Hel1o/ });
  });

  it("clicks a word to open the inline editor (without Font toolbar or render patch), validates, cancels on Esc, and commits on Enter", async () => {
    const { editing, ocrApi, renderer, rerenderDoc, user } = await setup();

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: "修正辨識的文字" }));

    const firstWordBtn = await screen.findByRole("button", { name: /Hel1o/ });
    const renderCallsBefore = vi.mocked(renderer.render).mock.calls.length;

    await user.click(firstWordBtn);

    // Uses InlineTextEditor inline (no modal dialog, no FreeText formatting toolbar, no withoutAnnotations patch).
    const textbox = await screen.findByRole("textbox", { name: "修正辨識的文字" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("toolbar", { name: strings.annotations.textBoxEditor.toolbarLabel })).toBeNull();
    expect(screen.getByText("只改搜尋與複製用的字，畫面不會改變")).toBeInTheDocument();
    expect(textbox).toHaveFocus();
    expect(textbox).toHaveValue("Hel1o");
    expect(vi.mocked(renderer.render).mock.calls.length).toBe(renderCallsBefore);

    await expectAccessible(document.body);

    // IME composing Enter / Escape belong to the input method and do not commit or close.
    fireEvent.compositionStart(textbox);
    fireEvent.keyDown(textbox, { key: "Enter", isComposing: true });
    fireEvent.keyDown(textbox, { key: "Escape", isComposing: true });
    expect(screen.getByRole("textbox", { name: "修正辨識的文字" })).toBeInTheDocument();
    expect(editing.applyEdit).not.toHaveBeenCalled();
    fireEvent.compositionEnd(textbox);

    // Esc cancels without editing and restores focus to the word button (staying in correction mode).
    await user.clear(textbox);
    await user.type(textbox, "TypedButCancelled");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "修正辨識的文字" })).toBeNull());
    expect(editing.applyEdit).not.toHaveBeenCalled();
    await waitFor(() => expect(firstWordBtn).toHaveFocus());

    // Tab moves to the next word button; Enter opens inline editor on it.
    await user.tab();
    const secondWordBtn = screen.getByRole("button", { name: /W0rld/ });
    expect(secondWordBtn).toHaveFocus();
    await user.keyboard("{Enter}");

    const secondInput = await screen.findByRole("textbox", { name: "修正辨識的文字" });
    expect(secondInput).toHaveValue("W0rld");

    // Empty input shows alert and blocks commit.
    await user.clear(secondInput);
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(editing.applyEdit).not.toHaveBeenCalled();

    // Oversized input shows alert and blocks commit.
    fireEvent.change(secondInput, { target: { value: "a".repeat(LIMITS.maxRecognisedWordBytes + 1) } });
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(editing.applyEdit).not.toHaveBeenCalled();

    // Control character shows alert and blocks commit.
    fireEvent.change(secondInput, { target: { value: "Bad\u0007" } });
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(editing.applyEdit).not.toHaveBeenCalled();

    // Valid fix + Enter commits one Edit::CorrectRecognisedText without closing mode.
    await user.clear(secondInput);
    await user.type(secondInput, "World{Enter}");
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, {
      kind: "correctRecognisedText",
      page: 0,
      word: 1,
      expected: "W0rld",
      text: "World",
    });
    expect(lastEdit(editing)).toEqual({
      kind: "correctRecognisedText",
      page: 0,
      word: 1,
      expected: "W0rld",
      text: "World",
    });
    await waitFor(() => expect(screen.queryByRole("textbox", { name: "修正辨識的文字" })).toBeNull());

    // Simulating doc update after commit re-fetches recognised words for the new doc id.
    rerenderDoc(6);
    await waitFor(() => expect(ocrApi.getRecognisedWords).toHaveBeenCalledWith(6, 0));
  });

  it("shows server error alert and keeps editor open when applyEdit rejects", async () => {
    const { editing, user } = await setup({
      applyEdit: () => Promise.reject({ code: "invalidArgument", message: "text mismatch" }),
    });
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: "修正辨識的文字" }));
    await user.click(await screen.findByRole("button", { name: /Hel1o/ }));
    const textbox = await screen.findByRole("textbox", { name: "修正辨識的文字" });
    await user.clear(textbox);
    await user.type(textbox, "Hello");
    await user.click(screen.getByRole("button", { name: "確定" }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "修正辨識的文字" })).toBeInTheDocument();
    expect(editing.applyEdit).toHaveBeenCalledTimes(1);
  });

  it("disables the menu and palette entry with reason when signed or when modify permission is denied", async () => {
    // 1. Signed document
    const signedSetup = await setup({ signatures: SIGNED_VIEW });
    await signedSetup.user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    const signedMenu = await screen.findByRole("menuitem", { name: /修正辨識的文字/ });
    expect(signedMenu).toHaveAttribute("aria-disabled", "true");
    expect(signedMenu.textContent).toContain("簽章");
    await signedSetup.user.keyboard("{Escape}");

    await signedSetup.user.keyboard("{Control>}k{/Control}");
    const signedSearch = await screen.findByPlaceholderText(strings.commands.palettePlaceholder);
    await signedSetup.user.type(signedSearch, "修正辨識的文字");
    const signedCmd = await screen.findByRole("option", { name: /修正辨識的文字/ });
    expect(signedCmd).toHaveAttribute("aria-disabled", "true");
    signedSetup.unmount();

    // 2. Author forbids modify
    const blockedSetup = await setup({ permissions: { ...ALL_PERMISSIONS, modify: false } });
    await blockedSetup.user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    const blockedMenu = await screen.findByRole("menuitem", { name: /修正辨識的文字/ });
    expect(blockedMenu).toHaveAttribute("aria-disabled", "true");
    expect(blockedMenu.textContent).toContain(strings.permissions.notAllowed);
    expect(blockedSetup.ocrApi.getRecognisedWords).not.toHaveBeenCalled();
  });

  it("explains clearly on unusable pages (especially memoryOcrOnly, noTextLayer, mixedRenderModes)", async () => {
    const reasons: [RecognisedWordsPage, string][] = [
      [{ status: "unusable", reason: "memoryOcrOnly" }, "先另存為可搜尋的 PDF，再打開副本修正"],
      [{ status: "unusable", reason: "noTextLayer" }, "沒有"],
      [{ status: "unusable", reason: "mixedRenderModes" }, "混合"],
    ];

    for (const [response, expectedSnippet] of reasons) {
      const { unmount, user } = await setup({
        getWords: () => Promise.resolve(response),
      });
      await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
      await user.click(await screen.findByRole("menuitem", { name: "修正辨識的文字" }));

      await waitFor(() =>
        expect(screen.getByRole("status").textContent || screen.getByRole("contentinfo").textContent).toContain(
          expectedSnippet,
        ),
      );
      expect(screen.queryByRole("button", { name: /Hel1o/ })).toBeNull();
      unmount();
    }
  });
});
