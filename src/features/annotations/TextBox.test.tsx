import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { type AnnotationsApi } from "@/features/annotations/source";
import { textBoxRectAt } from "@/features/annotations/tools";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { contentWidth, layoutPages, pageLeft } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import type { Edit, FontInfo, PageAnnotation, TextBoxInfo } from "@/ipc/generated/contract";

const t = strings.annotations;
const d = t.textBoxDialog;

const font = (id: number, name: string, family: string, bold = false, italic = false): FontInfo => ({
  id,
  name,
  family,
  bold,
  italic,
  source: "installed",
});
/** Arial has all four faces, JhengHei a bold one, Segoe Script only its regular. */
const FONTS: FontInfo[] = [
  font(1, "Arial", "Arial"),
  font(2, "Arial Bold", "Arial", true),
  font(3, "Arial Italic", "Arial", false, true),
  font(4, "Arial Bold Italic", "Arial", true, true),
  font(5, "Microsoft JhengHei", "Microsoft JhengHei"),
  font(6, "Microsoft JhengHei Bold", "Microsoft JhengHei", true),
  font(7, "Segoe Script", "Segoe Script"),
];

const box: PageAnnotation = {
  id: 12,
  kind: "textBox",
  rect: { x0: 100, y0: 200, x1: 340, y1: 248 },
  color: null,
  picture: null,
  text: "Hello 你好",
};
const INFO: TextBoxInfo = {
  text: "Hello 你好",
  font: "Arial Bold",
  style: { size: 18, color: { r: 200, g: 30, b: 30 }, align: "center" },
  viewOnly: false,
  overflow: false,
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
  options: { onPage?: PageAnnotation[]; info?: TextBoxInfo; prepare?: AnnotationsApi["prepareTextBox"] } = {},
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
    prepareTextBox: vi.fn(options.prepare ?? (() => Promise.resolve({ model: 7, viewOnly: false }))),
    getTextBox: vi.fn(() => Promise.resolve(options.info ?? INFO)),
  } satisfies AnnotationsApi;
  render(
    <ReaderShell
      state={{ kind: "open", document: { ...demoDocument, doc: 5, permissions } }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      editingApi={editing}
      annotationsApi={annotationsApi}
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
  const dialog = () => screen.findByRole("dialog");
  const pages = screen.getByRole("img", { name: "第 1 頁" }).parentElement!;
  return { annotationsApi, editing, layer, toolButton, dialog, pages, user };
}

const lastEdit = (editing: { applyEdit: ReturnType<typeof vi.fn> }): Edit => editing.applyEdit.mock.calls.at(-1)![1] as Edit;

describe("putting down a text box (B2-30)", () => {
  it("asks what it says where the page was clicked, and puts it there when saved", async () => {
    const { annotationsApi, editing, layer, toolButton, dialog, user } = await setup();
    expect(layer()).toBeNull();
    await user.click(toolButton());
    expect(toolButton()).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.textBoxOn);

    fireEvent.pointerDown(layer()!, pointer(100, 100));
    fireEvent.pointerUp(layer()!, pointer(100, 100));
    const opened = within(await dialog());
    expect(opened.getByRole("heading", { name: d.addTitle })).toBeInTheDocument();
    // The tool is put away: one box at a time.
    expect(layer()).toBeNull();
    expect(annotationsApi.getFonts).toHaveBeenCalledTimes(1);
    // It starts with a family that covers Chinese, in the default size, black, at the left.
    expect(opened.getByLabelText(d.font)).toHaveValue("Microsoft JhengHei");
    expect(opened.getByLabelText(d.size)).toHaveValue(14);
    expect(opened.getByRole("radio", { name: d.colors.black })).toBeChecked();
    expect(opened.getByRole("radio", { name: d.aligns.left })).toBeChecked();

    await user.type(opened.getByLabelText(d.text), "你好，世界");
    await user.click(opened.getByRole("button", { name: d.save }));

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(annotationsApi.prepareTextBox).toHaveBeenCalledWith(5, "你好，世界", 5);
    expect(lastEdit(editing)).toEqual({
      kind: "addTextBox",
      page: 0,
      rect: textBoxRectAt({ x: 100, y: 100 }, demoDocument.pages[0]!),
      model: 7,
      style: { size: 14, color: { r: 0, g: 0, b: 0 }, align: "left" },
    });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("takes the font, the variant, the size, the colour and the alignment the user chose", async () => {
    const { editing, layer, toolButton, dialog, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const opened = within(await dialog());
    await user.type(opened.getByLabelText(d.text), "Title");
    await user.selectOptions(opened.getByLabelText(d.font), "Arial");
    await user.click(opened.getByRole("button", { name: d.bold }));
    await user.click(opened.getByRole("button", { name: d.italic }));
    expect(opened.getByRole("button", { name: d.bold })).toHaveAttribute("aria-pressed", "true");
    await user.clear(opened.getByLabelText(d.size));
    await user.type(opened.getByLabelText(d.size), "24");
    await user.click(opened.getByRole("radio", { name: d.colors.red }));
    await user.click(opened.getByRole("radio", { name: d.aligns.right }));
    await user.click(opened.getByRole("button", { name: d.save }));

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    const edit = lastEdit(editing);
    expect(edit).toMatchObject({ kind: "addTextBox", style: { size: 24, color: { r: 200, g: 30, b: 30 }, align: "right" } });
  });

  it("says when a family has no bold or italic, and does not make one up", async () => {
    const { annotationsApi, layer, toolButton, dialog, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const opened = within(await dialog());
    await user.selectOptions(opened.getByLabelText(d.font), "Segoe Script");
    await user.click(opened.getByRole("button", { name: d.bold }));
    expect(opened.getByRole("status")).toHaveTextContent(d.noBold("Segoe Script"));
    expect(opened.getByRole("button", { name: d.bold })).toHaveAttribute("aria-pressed", "false");
    await user.click(opened.getByRole("button", { name: d.italic }));
    expect(opened.getByRole("status")).toHaveTextContent(d.noItalic("Segoe Script"));
    expect(opened.getByRole("button", { name: d.italic })).toHaveAttribute("aria-pressed", "false");

    // Bold on a family that has it; then a family that has not: the bold is dropped, with a note.
    await user.selectOptions(opened.getByLabelText(d.font), "Microsoft JhengHei");
    await user.click(opened.getByRole("button", { name: d.bold }));
    expect(opened.getByRole("button", { name: d.bold })).toHaveAttribute("aria-pressed", "true");
    await user.selectOptions(opened.getByLabelText(d.font), "Segoe Script");
    expect(opened.getByRole("button", { name: d.bold })).toHaveAttribute("aria-pressed", "false");
    expect(opened.getByRole("status")).toHaveTextContent(d.noBold("Segoe Script"));
    // The fonts were asked for once for this dialog, and the help is there for the missing one.
    expect(annotationsApi.getFonts).toHaveBeenCalledTimes(1);
    expect(opened.getByText(d.fontsHelp)).toBeInTheDocument();
  });

  it("keeps the dialog open and says why when the font cannot be used", async () => {
    const prepare = vi.fn<NonNullable<AnnotationsApi["prepareTextBox"]>>(() =>
      Promise.reject({ code: "fontMissingGlyphs", message: "no glyph" }),
    );
    const { editing, layer, toolButton, dialog, user } = await setup(ALL_PERMISSIONS, { prepare });
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const opened = within(await dialog());
    await user.type(opened.getByLabelText(d.text), "words");
    await user.click(opened.getByRole("button", { name: d.save }));
    expect(await opened.findByRole("alert")).toHaveTextContent(strings.error.messages.fontMissingGlyphs);
    expect(editing.applyEdit).not.toHaveBeenCalled();
    // And the user can pick another font and try again.
    prepare.mockResolvedValueOnce({ model: 9, viewOnly: false });
    await user.selectOptions(opened.getByLabelText(d.font), "Arial");
    await user.click(opened.getByRole("button", { name: d.save }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(lastEdit(editing)).toMatchObject({ kind: "addTextBox", model: 9 });
  });

  it("says in the status bar when the font may only be viewed and printed", async () => {
    const prepare = vi.fn<NonNullable<AnnotationsApi["prepareTextBox"]>>(() => Promise.resolve({ model: 3, viewOnly: true }));
    const { editing, layer, toolButton, dialog, user } = await setup(ALL_PERMISSIONS, { prepare });
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const opened = within(await dialog());
    await user.type(opened.getByLabelText(d.text), "words");
    await user.click(opened.getByRole("button", { name: d.save }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.textBoxViewOnly("Microsoft JhengHei"));
  });

  it("does not save words that are empty, too long, or in a size a box cannot have", async () => {
    const { annotationsApi, layer, toolButton, dialog, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const opened = within(await dialog());
    await user.click(opened.getByRole("button", { name: d.save }));
    expect(await opened.findByRole("alert")).toHaveTextContent(d.empty);
    fireEvent.change(opened.getByLabelText(d.text), { target: { value: "字".repeat(1400) } });
    await user.click(opened.getByRole("button", { name: d.save }));
    expect(await opened.findByRole("alert")).toHaveTextContent(d.tooLong);
    fireEvent.change(opened.getByLabelText(d.text), { target: { value: "ok" } });
    fireEvent.change(opened.getByLabelText(d.size), { target: { value: "500" } });
    await user.click(opened.getByRole("button", { name: d.save }));
    expect(await opened.findByRole("alert")).toHaveTextContent(d.sizeRange(4, 288));
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
  });

  it("is not triggered by the keys of an input method, which are the input method's", async () => {
    const { annotationsApi, editing, layer, toolButton, dialog, user } = await setup();
    await user.click(toolButton());
    fireEvent.pointerDown(layer()!, pointer(300, 300));
    fireEvent.pointerUp(layer()!, pointer(300, 300));
    const opened = within(await dialog());
    const field = opened.getByLabelText(d.text);
    fireEvent.change(field, { target: { value: "你" } });
    // Composing: Ctrl+Enter confirms a candidate, it does not save.
    fireEvent.compositionStart(field);
    fireEvent.keyDown(field, { key: "Enter", ctrlKey: true, isComposing: true });
    fireEvent.keyDown(field, { key: "f", ctrlKey: true, isComposing: true });
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
    // No search bar was opened by the Ctrl+F of the composition.
    expect(screen.queryByRole("search")).toBeNull();
    fireEvent.compositionEnd(field);
    // A plain Enter is a line break in a text area, never a save.
    await user.click(field);
    await user.keyboard("{Enter}好");
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
    expect(field).toHaveValue("你\n好");
    // Ctrl+Enter, no longer composing, saves.
    fireEvent.keyDown(field, { key: "Enter", ctrlKey: true });
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(annotationsApi.prepareTextBox).toHaveBeenCalledWith(5, "你\n好", 5);
  });

  it("opens from the context menu, and with Enter for those who cannot point", async () => {
    const { pages, toolButton, dialog, user } = await setup();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    await user.click(await screen.findByRole("menuitem", { name: t.addTextBox }));
    expect(within(await dialog()).getByRole("heading", { name: d.addTitle })).toBeInTheDocument();
    await user.click(within(await dialog()).getByRole("button", { name: d.cancel }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    // With the tool on, Enter asks for a box in the middle of the page being read.
    await user.click(toolButton());
    await user.keyboard("{Enter}");
    expect(within(await dialog()).getByRole("heading", { name: d.addTitle })).toBeInTheDocument();
  });

  it("is off where the author does not allow annotating", async () => {
    const { annotationsApi, toolButton, pages, user } = await setup({ ...ALL_PERMISSIONS, annotate: false });
    expect(toolButton()).toBeDisabled();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    const item = await screen.findByRole("menuitem", { name: new RegExp(t.addTextBox) });
    expect(item).toHaveAttribute("aria-disabled", "true");
    await user.keyboard("{Escape}");
    expect(annotationsApi.getFonts).not.toHaveBeenCalled();
  });
});

describe("changing a text box (B2-30)", () => {
  const choose = async () => {
    const outline = await screen.findByRole("button", { name: t.textBoxSaying("Hello 你好") });
    outline.focus();
    return outline;
  };

  it("starts the dialog from what the box says, and sends the words only when they changed", async () => {
    const { annotationsApi, editing, dialog, user } = await setup();
    await choose();
    await user.click(await screen.findByRole("button", { name: t.editTextBox }));
    const opened = within(await dialog());
    expect(opened.getByRole("heading", { name: d.editTitle })).toBeInTheDocument();
    expect(annotationsApi.getTextBox).toHaveBeenCalledWith(5, 0, 12);
    expect(opened.getByLabelText(d.text)).toHaveValue("Hello 你好");
    expect(opened.getByLabelText(d.font)).toHaveValue("Arial");
    expect(opened.getByRole("button", { name: d.bold })).toHaveAttribute("aria-pressed", "true");
    expect(opened.getByLabelText(d.size)).toHaveValue(18);
    expect(opened.getByRole("radio", { name: d.colors.red })).toBeChecked();
    expect(opened.getByRole("radio", { name: d.aligns.center })).toBeChecked();

    // Only the size: the words and the font are the box's own, nothing is prepared.
    await user.clear(opened.getByLabelText(d.size));
    await user.type(opened.getByLabelText(d.size), "30");
    await user.click(opened.getByRole("button", { name: d.save }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
    expect(lastEdit(editing)).toEqual({
      kind: "setTextBox",
      page: 0,
      annotation: 12,
      model: null,
      style: { size: 30, color: { r: 200, g: 30, b: 30 }, align: "center" },
    });
  });

  it("prepares the words again when the text or the font changed", async () => {
    const { annotationsApi, editing, dialog, user } = await setup();
    await choose();
    await user.click(await screen.findByRole("button", { name: t.editTextBox }));
    const opened = within(await dialog());
    await user.type(opened.getByLabelText(d.text), "!");
    await user.click(opened.getByRole("button", { name: d.save }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(annotationsApi.prepareTextBox).toHaveBeenCalledWith(5, "Hello 你好!", 2);
    expect(lastEdit(editing)).toMatchObject({ kind: "setTextBox", annotation: 12, model: 7 });
  });

  it("sends nothing when nothing changed", async () => {
    const { annotationsApi, editing, dialog, user } = await setup();
    await choose();
    await user.click(await screen.findByRole("button", { name: t.editTextBox }));
    const opened = within(await dialog());
    await user.click(opened.getByRole("button", { name: d.save }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(editing.applyEdit).not.toHaveBeenCalled();
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
  });

  it("keeps a box whose font is gone as it is, and asks for another font only for other words", async () => {
    const gone: TextBoxInfo = { ...INFO, font: "Gone Font" };
    const { annotationsApi, editing, dialog, user } = await setup(ALL_PERMISSIONS, { info: gone });
    await choose();
    await user.click(await screen.findByRole("button", { name: t.editTextBox }));
    const opened = within(await dialog());
    expect(opened.getByLabelText(d.font)).toHaveValue("");
    expect(opened.getByRole("option", { name: d.fontMissing("Gone Font") })).toBeInTheDocument();

    // Other words need a font that is there.
    await user.type(opened.getByLabelText(d.text), "!");
    await user.click(opened.getByRole("button", { name: d.save }));
    expect(await opened.findByRole("alert")).toHaveTextContent(d.needFont);
    expect(annotationsApi.prepareTextBox).not.toHaveBeenCalled();
    // The colour does not.
    await user.clear(opened.getByLabelText(d.text));
    await user.type(opened.getByLabelText(d.text), "Hello 你好");
    await user.click(opened.getByRole("radio", { name: d.colors.blue }));
    await user.click(opened.getByRole("button", { name: d.save }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(lastEdit(editing)).toMatchObject({ kind: "setTextBox", model: null, style: { color: { r: 26, g: 77, b: 217 } } });
  });

  it("tells that the words do not fit the box", async () => {
    const { dialog, user } = await setup(ALL_PERMISSIONS, { info: { ...INFO, overflow: true } });
    await choose();
    await user.click(await screen.findByRole("button", { name: t.editTextBox }));
    expect(within(await dialog()).getByText(d.overflow)).toBeInTheDocument();
  });

  it("can be changed, moved and removed with the keyboard alone", async () => {
    const { editing, dialog, user } = await setup();
    const outline = await choose();
    // Enter asks for the words; the arrow keys move the box a point, ten with Shift; Delete removes it.
    await user.keyboard("{Enter}");
    expect(within(await dialog()).getByRole("heading", { name: d.editTitle })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    outline.focus();
    await user.keyboard("{ArrowRight}");
    expect(lastEdit(editing)).toMatchObject({
      kind: "setAnnotationRect",
      page: 0,
      annotation: 12,
      rect: { x0: expect.closeTo(101, 3), y0: expect.closeTo(200, 3), x1: expect.closeTo(341, 3), y1: expect.closeTo(248, 3) },
    });
    await user.keyboard("{Shift>}{ArrowDown}{/Shift}");
    expect(lastEdit(editing)).toMatchObject({
      kind: "setAnnotationRect",
      rect: { y0: expect.closeTo(210, 3), y1: expect.closeTo(258, 3) },
    });
    await user.keyboard("{Delete}");
    expect(lastEdit(editing)).toEqual({ kind: "deleteAnnotation", page: 0, annotation: 12 });
  });

  it("can be resized by its handles like a drawing, but not kept in shape like a stamp", async () => {
    const { editing } = await setup();
    await choose();
    const area = await waitFor(() => {
      const found = document.querySelector<HTMLElement>('[data-move-resize="12"]');
      expect(found).not.toBeNull();
      return found!;
    });
    // All eight handles: a text box is resized freely.
    expect(area.querySelectorAll("[data-handle]")).toHaveLength(8);
    const east = area.querySelector<HTMLElement>('[data-handle="e"]')!;
    fireEvent.pointerDown(east, { pointerId: 2, button: 0, clientX: 400, clientY: 300 });
    fireEvent.pointerMove(east, { pointerId: 2, clientX: 520, clientY: 300 });
    fireEvent.pointerUp(east, { pointerId: 2, clientX: 520, clientY: 300 });
    const edit = lastEdit(editing);
    expect(edit).toMatchObject({ kind: "setAnnotationRect", annotation: 12 });
    if (edit.kind !== "setAnnotationRect") throw new Error("a resize");
    // 120 pixels wider is 90 points; the top and the bottom stay where they were.
    expect(edit.rect.x1 - edit.rect.x0).toBeCloseTo(240 + 90, 0);
    expect(edit.rect.y0).toBeCloseTo(200, 3);
    expect(edit.rect.y1).toBeCloseTo(248, 3);
  });
});
