import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { type AnnotationsApi } from "@/features/annotations/source";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import type { SavingApi } from "@/features/saving/api";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { contentWidth, layoutPages, pageLeft } from "@/features/viewer/layout";
import type { PageRenderer } from "@/features/viewer/renderer";
import { strings } from "@/i18n";
import type { Edit, FontInfo, PageAnnotation, TextBoxInfo } from "@/ipc/generated/contract";

const t = strings.annotations;
const e = t.textBoxEditor;

const font = (
  id: number,
  name: string,
  family: string,
  bold = false,
  italic = false,
  source: FontInfo["source"] = "installed",
): FontInfo => ({
  id,
  name,
  family,
  bold,
  italic,
  source,
});

/** Arial has all four faces, JhengHei a bold one, Segoe Script only its regular, Custom Sans is imported. */
const FONTS: FontInfo[] = [
  font(1, "Arial", "Arial"),
  font(2, "Arial Bold", "Arial", true),
  font(3, "Arial Italic", "Arial", false, true),
  font(4, "Arial Bold Italic", "Arial", true, true),
  font(5, "Microsoft JhengHei", "Microsoft JhengHei"),
  font(6, "Microsoft JhengHei Bold", "Microsoft JhengHei", true),
  font(7, "Segoe Script", "Segoe Script"),
  font(8, "Custom Sans", "Custom Sans", false, false, "imported"),
];

const box: PageAnnotation = {
  id: 12,
  kind: "textBox",
  rect: { x0: 100, y0: 200, x1: 340, y1: 248 },
  color: null,
  picture: null,
  text: "Hello 你好",
  lineEnds: null,
  shapeColor: null,
  shapeWidth: null,
};

const INFO: TextBoxInfo = {
  text: "Hello 你好",
  font: "Arial Bold",
  style: { size: 18, color: { r: 200, g: 30, b: 30 }, align: "center" },
  viewOnly: false,
  overflow: false,
  lineStep: 1200,
};

/** The pointer on page 1, `x` and `y` in page points: 4 / 3 CSS pixels each at 100%, from the layer's corner. */
const pointer = (x: number, y: number) => ({ pointerId: 1, button: 0, clientX: (x * 4) / 3, clientY: (y * 4) / 3 });

/** Client coordinates of a point on page `index`, in page points, at 100%. */
const at = (index: number, x: number, y: number) => {
  const layout = layoutPages(demoDocument.pages, 0, 1);
  const place = layout.boxes[index]!;
  return { clientX: pageLeft(place, contentWidth(layout, 1000)) + (x * 4) / 3, clientY: place.top + (y * 4) / 3 };
};

async function setup(
  permissions: DocumentPermissions = ALL_PERMISSIONS,
  options: {
    onPage?: PageAnnotation[];
    info?: TextBoxInfo;
    prepare?: AnnotationsApi["prepareTextBox"];
    savingApi?: SavingApi;
    renderer?: PageRenderer;
    unsaved?: boolean;
  } = {},
) {
  const editing = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
  const onPage = options.onPage ?? [box];
  const annotationsApi = {
    getPageAnnotations: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? onPage : [])),
    getFonts: vi.fn(() => Promise.resolve(FONTS)),
    prepareTextBox: vi.fn(
      options.prepare ?? (() => Promise.resolve({ model: 7, viewOnly: false, lineStep: 1250 })),
    ),
    getTextBox: vi.fn(() => Promise.resolve(options.info ?? INFO)),
  } satisfies AnnotationsApi;
  const renderer: PageRenderer = options.renderer ?? {
    render: vi.fn(() => ({
      result: Promise.resolve({ width: 10, height: 10, pixels: new Uint8ClampedArray(400) }),
      cancel: vi.fn(),
    })),
  };
  const renderResult = render(
    <ReaderShell
      state={{
        kind: "open",
        document: { ...demoDocument, doc: 5, permissions, unsaved: options.unsaved ?? true },
      }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      editingApi={editing}
      annotationsApi={annotationsApi}
      savingApi={options.savingApi}
      renderer={renderer}
    />,
  );
  const user = userEvent.setup({ pointerEventsCheck: PointerEventsCheckLevel.Never });
  await user.keyboard("{Control>}1{/Control}");
  const canvas = screen.getByRole("main");
  Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
  Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 800 });
  act(() => canvas.dispatchEvent(new Event("scroll")));
  await waitFor(() => expect(annotationsApi.getPageAnnotations).toHaveBeenCalledWith(5, 0));
  await act(async () => {});
  const layer = () => document.querySelector<HTMLElement>('[data-page-drawing="1"]');
  const toolButton = () => screen.getByRole("button", { name: t.textBox });
  const editor = () => screen.findByRole("textbox", { name: e.label });
  const toolbar = () => screen.findByRole("toolbar", { name: e.toolbarLabel });
  const pages = screen.getByRole("img", { name: "第 1 頁" }).parentElement!;
  return { ...renderResult, annotationsApi, editing, renderer, layer, toolButton, editor, toolbar, pages, user };
}

const lastEdit = (editing: { applyEdit: ReturnType<typeof vi.fn> }): Edit => editing.applyEdit.mock.calls.at(-1)![1] as Edit;

describe("typing a new text box directly on the page (#719, B2-30)", () => {
  it("opens an inline editor on the page without a modal dialog, and commits on click outside", async () => {
    const { annotationsApi, editing, layer, toolButton, editor, toolbar, pages, user } = await setup();
    expect(layer()).toBeNull();
    await user.click(toolButton());
    expect(toolButton()).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.textBoxOn);

    fireEvent.pointerDown(layer()!, pointer(100, 100));
    fireEvent.pointerUp(layer()!, pointer(100, 100));
    const textarea = await editor();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(textarea).toHaveFocus();
    expect(layer()).toBeNull();

    const bar = within(await toolbar());
    // Imported fonts appear first in the dropdown with the imported badge.
    const options = bar.getByLabelText(e.font).querySelectorAll("option");
    expect(options[0]).toHaveTextContent(`Custom Sans${e.importedBadge}`);
    // Default family covers Chinese, size 14, black, left.
    expect(bar.getByLabelText(e.font)).toHaveValue("Microsoft JhengHei");
    expect(bar.getByLabelText(e.size)).toHaveValue(14);
    expect(bar.getByRole("radio", { name: e.colors.black })).toBeChecked();
    expect(bar.getByRole("radio", { name: e.aligns.left })).toBeChecked();

    await user.type(textarea, "你好，世界");
    // Click outside the box and toolbar commits in one edit.
    fireEvent.pointerDown(pages);

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(annotationsApi.prepareTextBox).toHaveBeenCalledWith(5, "你好，世界", 5);
    expect(lastEdit(editing)).toMatchObject({
      kind: "addTextBox",
      page: 0,
      model: 7,
      style: { size: 14, color: { r: 0, g: 0, b: 0 }, align: "left" },
    });
    await waitFor(() => expect(screen.queryByRole("textbox", { name: e.label })).toBeNull());
  });

  it("commits nothing when an empty new box is dismissed", async () => {
    const { annotationsApi, editing, layer, toolButton, editor, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(100, 100));
    fireEvent.pointerUp(layer()!, pointer(100, 100));
    await editor();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("textbox", { name: e.label })).toBeNull());
    expect(editing.applyEdit).not.toHaveBeenCalled();
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
  });

  it("applies chosen font, variant, size, colour, and alignment from the floating toolbar", async () => {
    const { editing, layer, toolButton, editor, toolbar, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const textarea = await editor();
    const bar = within(await toolbar());

    await user.type(textarea, "Title");
    await user.selectOptions(bar.getByLabelText(e.font), "Arial");
    await user.click(bar.getByRole("button", { name: e.bold }));
    await user.click(bar.getByRole("button", { name: e.italic }));
    expect(bar.getByRole("button", { name: e.bold })).toHaveAttribute("aria-pressed", "true");
    await user.clear(bar.getByLabelText(e.size));
    await user.type(bar.getByLabelText(e.size), "24");
    await user.click(bar.getByRole("radio", { name: e.colors.red }));
    await user.click(bar.getByRole("radio", { name: e.aligns.right }));
    await user.keyboard("{Escape}");

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(lastEdit(editing)).toMatchObject({
      kind: "addTextBox",
      style: { size: 24, color: { r: 200, g: 30, b: 30 }, align: "right" },
    });
  });

  it("shows fallback font preview notice for imported fonts and refuses synthetic bold/italic", async () => {
    const { layer, toolButton, editor, toolbar, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    await editor();
    const bar = within(await toolbar());

    await user.selectOptions(bar.getByLabelText(e.font), "Custom Sans");
    expect(bar.getByRole("status")).toHaveTextContent(e.fallbackFontNote("Custom Sans"));

    await user.selectOptions(bar.getByLabelText(e.font), "Segoe Script");
    await user.click(bar.getByRole("button", { name: e.bold }));
    expect(bar.getByRole("status")).toHaveTextContent(e.noBold("Segoe Script"));
    expect(bar.getByRole("button", { name: e.bold })).toHaveAttribute("aria-pressed", "false");
    await user.click(bar.getByRole("button", { name: e.italic }));
    expect(bar.getByRole("status")).toHaveTextContent(e.noItalic("Segoe Script"));
    expect(bar.getByRole("button", { name: e.italic })).toHaveAttribute("aria-pressed", "false");
    expect(bar.getByText(e.fontsHelp)).toBeInTheDocument();
  });

  it("keeps editing open and shows alert when font lacks glyphs, then commits after switching font", async () => {
    const prepare = vi.fn<NonNullable<AnnotationsApi["prepareTextBox"]>>(() =>
      Promise.reject({ code: "fontMissingGlyphs", message: "no glyph" }),
    );
    const { editing, layer, toolButton, editor, toolbar, user } = await setup(ALL_PERMISSIONS, { prepare });
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const textarea = await editor();
    const bar = within(await toolbar());

    await user.type(textarea, "words");
    await user.keyboard("{Escape}");
    expect(await bar.findByRole("alert")).toHaveTextContent(strings.error.messages.fontMissingGlyphs);
    expect(editing.applyEdit).not.toHaveBeenCalled();

    prepare.mockResolvedValue({ model: 9, viewOnly: false, lineStep: 1200 });
    await user.selectOptions(bar.getByLabelText(e.font), "Arial");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(lastEdit(editing)).toMatchObject({ kind: "addTextBox", model: 9 });
  });

  it("validates max length and size range before committing", async () => {
    const { annotationsApi, layer, toolButton, editor, toolbar, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const textarea = await editor();
    const bar = within(await toolbar());

    fireEvent.change(textarea, { target: { value: "字".repeat(1400) } });
    await user.keyboard("{Escape}");
    expect(await bar.findByRole("alert")).toHaveTextContent(e.tooLong);

    fireEvent.change(textarea, { target: { value: "ok" } });
    fireEvent.change(bar.getByLabelText(e.size), { target: { value: "500" } });
    await user.keyboard("{Escape}");
    expect(await bar.findByRole("alert")).toHaveTextContent(e.sizeRange(4, 288));
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
  });

  it("handles IME composition, keeps editing keys inside textarea, and commits before Ctrl+S", async () => {
    const savingApi = {
      save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false, redaction: null })),
      saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
      privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(false)),
      encryptCopy: vi.fn<SavingApi["encryptCopy"]>(() => Promise.resolve(false)),
      decryptCopy: vi.fn<SavingApi["decryptCopy"]>(() => Promise.resolve(false)),
      compressImages: vi.fn<SavingApi["compressImages"]>(() => Promise.resolve(null)),
      pageMarksCopy: vi.fn<SavingApi["pageMarksCopy"]>(() => Promise.resolve(false)),
      closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    } satisfies SavingApi;
    const { editing, layer, toolButton, editor, user } = await setup(ALL_PERMISSIONS, { savingApi });
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const textarea = await editor();

    fireEvent.change(textarea, { target: { value: "你" } });
    // Composing: Enter / Escape / Ctrl+F belong to the input method.
    fireEvent.compositionStart(textarea);
    fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });
    fireEvent.keyDown(textarea, { key: "Escape", isComposing: true });
    fireEvent.keyDown(textarea, { key: "f", ctrlKey: true, isComposing: true });
    expect(screen.queryByRole("search")).toBeNull();
    expect(editing.applyEdit).not.toHaveBeenCalled();
    fireEvent.compositionEnd(textarea);

    // Plain Enter inserts a newline; Delete/Backspace/Arrow/Ctrl+Z do not trigger app undo or box edits.
    await user.keyboard("{Enter}好");
    expect(textarea).toHaveValue("你\n好");
    await user.keyboard("{ArrowLeft}{Delete}{Control>}z{/Control}");
    expect(editing.undo).not.toHaveBeenCalled();
    expect(editing.applyEdit).not.toHaveBeenCalled();

    // Ctrl+S commits the inline text box first, then saves the document.
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(savingApi.save).toHaveBeenCalledTimes(1));
  });

  it("opens inline editor from context menu and via tool button + Enter, and stays off when forbidden", async () => {
    const { pages, toolButton, editor, user } = await setup();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    await user.click(await screen.findByRole("menuitem", { name: t.addTextBox }));
    expect(await editor()).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("textbox", { name: e.label })).toBeNull());

    await user.click(toolButton());
    await user.keyboard("{Enter}");
    expect(await editor()).toBeInTheDocument();
    await user.keyboard("{Escape}");
  });

  it("measures wrapped height at the widened width when filling long text into a new box", async () => {
    const widthSpy = vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockImplementation(function (
      this: HTMLElement,
    ) {
      return this.dataset.textBoxMirror === "width" && (this.textContent ?? "").includes("長段") ? 600 : 0;
    });
    const heightSpy = vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function (
      this: HTMLElement,
    ) {
      if (this.dataset.textBoxMirror !== "height" || !(this.textContent ?? "").includes("長段")) return 0;
      const widthPx = parseFloat(this.style.width) || 0;
      return widthPx < 300 ? 160 : 56;
    });
    try {
      const { editing, layer, toolButton, editor, pages, user } = await setup();
      await user.click(toolButton());
      fireEvent.pointerDown(layer()!, pointer(100, 100));
      fireEvent.pointerUp(layer()!, pointer(100, 100));
      const textarea = await editor();

      fireEvent.change(textarea, { target: { value: "長段測試文字" } });
      fireEvent.pointerDown(pages);

      await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
      const edit = lastEdit(editing);
      expect(edit.kind).toBe("addTextBox");
      if (edit.kind === "addTextBox") {
        expect(edit.rect.x1 - edit.rect.x0).toBeGreaterThan(400);
        // Must reflect the widened height (~43.5 pt), not the stale 120 pt-width height (~121.5 pt).
        expect(edit.rect.y1 - edit.rect.y0).toBeLessThan(60);
      }
    } finally {
      widthSpy.mockRestore();
      heightSpy.mockRestore();
    }
  });

  it("cancels pending background probe when committing before debounce expires", async () => {
    const prepare = vi.fn<NonNullable<AnnotationsApi["prepareTextBox"]>>(
      () =>
        new Promise((resolve) => {
          window.setTimeout(() => resolve({ model: 7, viewOnly: false, lineStep: 1250 }), 550);
        }),
    );
    const { editing, layer, toolButton, editor, pages, user } = await setup(ALL_PERMISSIONS, { prepare });
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(100, 100));
    fireEvent.pointerUp(layer()!, pointer(100, 100));
    const textarea = await editor();

    fireEvent.change(textarea, { target: { value: "简体字测试" } });
    // Commit before the 400ms probe debounce expires; prepare takes 550ms (> 400ms).
    fireEvent.pointerDown(pages);

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1), { timeout: 2000 });
    expect(prepare).toHaveBeenCalledTimes(1);
    expect(lastEdit(editing)).toMatchObject({ kind: "addTextBox", model: 7 });
  });

  it("serializes commit after an already in-flight background probe instead of calling prepare_text_box concurrently", async () => {
    let resolveProbe!: (value: { model: number; viewOnly: boolean; lineStep: number }) => void;
    const probePending = new Promise<{ model: number; viewOnly: boolean; lineStep: number }>((resolve) => {
      resolveProbe = resolve;
    });
    const prepare = vi.fn<NonNullable<AnnotationsApi["prepareTextBox"]>>();
    prepare.mockReturnValueOnce(probePending);
    prepare.mockResolvedValueOnce({ model: 9, viewOnly: false, lineStep: 1250 });

    const { editing, layer, toolButton, editor, pages, user } = await setup(ALL_PERMISSIONS, { prepare });
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(100, 100));
    fireEvent.pointerUp(layer()!, pointer(100, 100));
    const textarea = await editor();

    fireEvent.change(textarea, { target: { value: "日本語のテスト" } });
    // Wait for the debounced background probe to start and remain in flight.
    await waitFor(() => expect(prepare).toHaveBeenCalledTimes(1), { timeout: 2000 });

    // Commit while the probe is still in flight: must wait for it rather than overlapping.
    fireEvent.pointerDown(pages);
    await new Promise((resolve) => window.setTimeout(resolve, 30));
    expect(prepare).toHaveBeenCalledTimes(1);

    resolveProbe({ model: 1, viewOnly: false, lineStep: 1300 });
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1), { timeout: 2000 });
    expect(prepare).toHaveBeenCalledTimes(2);
    expect(lastEdit(editing)).toMatchObject({ kind: "addTextBox", model: 9 });
  });

  it("is disabled when author forbids annotations", async () => {
    const { annotationsApi, toolButton, pages, user } = await setup({ ...ALL_PERMISSIONS, annotate: false });
    expect(toolButton()).toBeDisabled();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    const item = await screen.findByRole("menuitem", { name: new RegExp(t.addTextBox) });
    expect(item).toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Escape}");
    expect(annotationsApi.getFonts).not.toHaveBeenCalled();
  });
});

describe("editing and formatting an existing text box (#719, B2-30)", () => {
  const choose = async () => {
    const outline = await screen.findByRole("button", { name: t.textBoxSaying("Hello 你好") });
    outline.focus();
    return outline;
  };

  it("requests a withoutAnnotations region render patch while editing, and sends one setTextBox edit", async () => {
    const { annotationsApi, editing, renderer, editor, toolbar, user } = await setup();
    await choose();
    await user.click(await screen.findByRole("button", { name: t.editTextBox }));

    const textarea = await editor();
    expect(textarea).toHaveValue("Hello 你好");
    expect(annotationsApi.getTextBox).toHaveBeenCalledWith(5, 0, 12);

    // Background patch hides old FreeText glyphs without mutating the document.
    await waitFor(() =>
      expect(renderer.render).toHaveBeenCalledWith(
        expect.objectContaining({
          doc: 5,
          pageIndex: 0,
          withoutAnnotations: true,
          region: expect.objectContaining({ width: expect.any(Number), height: expect.any(Number) }),
        }),
      ),
    );
    expect(document.querySelector("[data-text-box-patch]")).not.toBeNull();

    const bar = within(await toolbar());
    expect(bar.getByLabelText(e.font)).toHaveValue("Arial");
    expect(bar.getByRole("button", { name: e.bold })).toHaveAttribute("aria-pressed", "true");
    expect(bar.getByLabelText(e.size)).toHaveValue(18);

    // Drag the right width handle while typing and append text: commits as ONE setTextBox edit with rect.
    const handle = document.querySelector<HTMLElement>('[data-inline-resize="e"]')!;
    fireEvent.pointerDown(handle, { pointerId: 3, button: 0, clientX: 300, clientY: 200 });
    fireEvent.pointerMove(handle, { pointerId: 3, clientX: 420, clientY: 200 });
    fireEvent.pointerUp(handle, { pointerId: 3, clientX: 420, clientY: 200 });

    await user.type(textarea, "!");
    await user.keyboard("{Escape}");

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(annotationsApi.prepareTextBox).toHaveBeenCalledWith(5, "Hello 你好!", 2);
    expect(lastEdit(editing)).toMatchObject({
      kind: "setTextBox",
      page: 0,
      annotation: 12,
      model: 7,
      rect: expect.objectContaining({ x0: 100, y0: 200 }),
    });
    // Patch disappears once editing finishes.
    await waitFor(() => expect(document.querySelector("[data-text-box-patch]")).toBeNull());
  });

  it("deletes an existing text box in one edit when cleared to empty", async () => {
    const { editing, editor, user } = await setup();
    await choose();
    await user.keyboard("{Enter}");
    const textarea = await editor();
    await user.clear(textarea);
    await user.keyboard("{Escape}");
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(lastEdit(editing)).toEqual({ kind: "deleteAnnotation", page: 0, annotation: 12 });
  });

  it("changes style directly from the selected floating toolbar without preparing or re-reading fonts", async () => {
    const { annotationsApi, editing, user } = await setup();
    await choose();
    const selectedBar = within(await screen.findByRole("toolbar", { name: t.textBoxSaying("Hello 你好") }));
    await waitFor(() => expect(selectedBar.getByLabelText(e.font)).toHaveValue("Arial"));

    const fontCallsBefore = annotationsApi.getFonts.mock.calls.length;
    await user.click(selectedBar.getByRole("radio", { name: e.colors.blue }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
    expect(annotationsApi.getFonts.mock.calls.length).toBe(fontCallsBefore);
    expect(lastEdit(editing)).toEqual({
      kind: "setTextBox",
      page: 0,
      annotation: 12,
      model: null,
      style: { size: 18, color: { r: 26, g: 77, b: 217 }, align: "center" },
    });

    // Changing font in selected mode prepares and sends one setTextBox with model.
    await user.selectOptions(selectedBar.getByLabelText(e.font), "Microsoft JhengHei");
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(2));
    expect(annotationsApi.prepareTextBox).toHaveBeenCalledWith(5, "Hello 你好", 6);
    expect(lastEdit(editing)).toMatchObject({ kind: "setTextBox", annotation: 12, model: 7 });
  });

  it("keeps a box whose font is gone: style change needs no font, text change asks for one", async () => {
    const gone: TextBoxInfo = { ...INFO, font: "Gone Font" };
    const { annotationsApi, editing, editor, toolbar, user } = await setup(ALL_PERMISSIONS, { info: gone });
    await choose();
    await user.keyboard("{Enter}");
    const textarea = await editor();
    const bar = within(await toolbar());
    expect(bar.getByLabelText(e.font)).toHaveValue("");
    expect(bar.getByRole("option", { name: e.fontMissing("Gone Font") })).toBeInTheDocument();

    await user.type(textarea, "!");
    await user.keyboard("{Escape}");
    expect(await bar.findByRole("alert")).toHaveTextContent(e.needFont);
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();

    // Restore original words and change colour: succeeds with model: null.
    await user.clear(textarea);
    await user.type(textarea, "Hello 你好");
    await user.click(bar.getByRole("radio", { name: e.colors.blue }));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(lastEdit(editing)).toMatchObject({
      kind: "setTextBox",
      model: null,
      style: { color: { r: 26, g: 77, b: 217 } },
    });
  });

  it("restores focus to the box outline after Escape (#402) and supports keyboard move and delete", async () => {
    const { editing, editor, user } = await setup();
    const outline = await choose();

    await user.keyboard("{Enter}");
    await editor();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("textbox", { name: e.label })).toBeNull());
    await waitFor(() => expect(outline).toHaveFocus());

    await user.keyboard("{ArrowRight}");
    expect(lastEdit(editing)).toMatchObject({
      kind: "setAnnotationRect",
      page: 0,
      annotation: 12,
      rect: { x0: expect.closeTo(101, 3), y0: expect.closeTo(200, 3), x1: expect.closeTo(341, 3), y1: expect.closeTo(248, 3) },
    });
    await user.keyboard("{Delete}");
    expect(lastEdit(editing)).toEqual({ kind: "deleteAnnotation", page: 0, annotation: 12 });
  });

  it("enters inline edit mode on double-click of a selected text box", async () => {
    const { editor } = await setup();
    await choose();
    const moveArea = await waitFor(() => {
      const el = document.querySelector<HTMLElement>('[data-move-resize="12"]');
      expect(el).not.toBeNull();
      return el!;
    });
    fireEvent.doubleClick(moveArea, { clientX: 200, clientY: 220 });
    expect(await editor()).toHaveValue("Hello 你好");
  });
});
