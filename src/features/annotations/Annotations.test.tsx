import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { annotationLabel } from "@/features/annotations/model";
import { PageAnnotations } from "@/features/annotations/PageAnnotations";
import { createAnnotationSource, type AnnotationsApi } from "@/features/annotations/source";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { TextApi } from "@/features/text/source";
import type { EditingApi } from "@/features/thumbnails/api";
import { contentWidth, layoutPages, pageLeft } from "@/features/viewer/layout";
import { strings } from "@/i18n/zh-TW";
import type { Edit, PageAnnotation, PageText } from "@/ipc/generated/contract";

const t = strings.annotations;

// "Hello world" 72 pt from the left and 100 pt from the top, 6 pt per character.
const hello: PageText = {
  lines: [
    {
      text: "Hello world",
      quad: { ul: { x: 72, y: 100 }, ur: { x: 138, y: 100 }, ll: { x: 72, y: 112 }, lr: { x: 138, y: 112 } },
      edges: Array.from({ length: 12 }, (_, i) => i * 6),
    },
  ],
  truncated: false,
};

const highlight: PageAnnotation = {
  id: 4,
  kind: "highlight",
  rect: { x0: 72, y0: 300, x1: 200, y1: 320 },
  color: "yellow",
  text: null,
};
const note: PageAnnotation = {
  id: 5,
  kind: "note",
  rect: { x0: 300, y0: 300, x1: 320, y1: 320 },
  color: null,
  text: "Existing note",
};

/** Client coordinates of a point on page `index`, in page points, at 100% (jsdom puts the view at 0, 0). */
const at = (index: number, x: number, y: number) => {
  const layout = layoutPages(demoDocument.pages, 0, 1);
  const box = layout.boxes[index]!;
  return { clientX: pageLeft(box, contentWidth(layout, 1000)) + (x * 4) / 3, clientY: box.top + (y * 4) / 3 };
};

async function setup(permissions: DocumentPermissions = ALL_PERMISSIONS, onPage: PageAnnotation[] = [highlight, note]) {
  const editing = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
  const textApi = {
    getPageText: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? hello : { lines: [], truncated: false })),
  } satisfies TextApi;
  const annotationsApi = {
    getPageAnnotations: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? onPage : [])),
  } satisfies AnnotationsApi;
  render(
    <ReaderShell
      state={{ kind: "open", document: { ...demoDocument, doc: 5, permissions } }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      editingApi={editing}
      textApi={textApi}
      annotationsApi={annotationsApi}
    />,
  );
  // The submenus' pop-ups let pointer events through only once they have settled: not in jsdom.
  const user = userEvent.setup({ pointerEventsCheck: PointerEventsCheckLevel.Never });
  // 100%: a page point is 4/3 CSS pixels.
  await user.keyboard("{Control>}1{/Control}");
  const canvas = screen.getByRole("main");
  Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
  Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 800 });
  act(() => canvas.dispatchEvent(new Event("scroll")));
  await waitFor(() => expect(textApi.getPageText).toHaveBeenCalledWith(5, 0));
  await waitFor(() => expect(annotationsApi.getPageAnnotations).toHaveBeenCalledWith(5, 0));
  await act(async () => {});
  const pages = screen.getByRole("img", { name: "第 1 頁" }).parentElement!;
  const selectHello = () => {
    fireEvent.mouseDown(pages, { button: 0, detail: 1, ...at(0, 73, 106) });
    fireEvent.mouseMove(window, at(0, 101, 106));
    fireEvent.mouseUp(window);
  };
  // The toolbar's button, not the outlines of the page's marks (named "螢光筆標示…").
  const highlightButton = () =>
    screen.getByRole("button", { name: new RegExp(`^${t.highlight}（.*）$|^${strings.toolbar.highlightNeedsText}$`) });
  return { editing, pages, selectHello, highlightButton, user };
}

/** The edit `applyEdit` was last called with. */
const lastEdit = (editing: { applyEdit: ReturnType<typeof vi.fn> }): Edit => editing.applyEdit.mock.calls.at(-1)![1] as Edit;

describe("the highlighter (B2-07)", () => {
  it("marks the selected text in the color picked from the context menu", async () => {
    const { editing, pages, selectHello, user } = await setup();
    selectHello();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    await user.click(await screen.findByRole("menuitem", { name: t.highlight }));
    fireEvent.click(await screen.findByRole("menuitem", { name: t.colors.green }));

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(editing.applyEdit).toHaveBeenCalledWith(5, {
      kind: "addHighlight",
      color: "green",
      marks: [{ page: 0, quads: [expect.objectContaining({ ul: { x: 72, y: 100 }, lr: expect.objectContaining({ y: 112 }) })] }],
    });
    const edit = lastEdit(editing);
    // "Hello" is five characters of six points.
    expect(edit.kind === "addHighlight" && edit.marks[0]!.quads[0]!.ur.x).toBe(102);
  });

  it("is on the toolbar too, in the color used last, once some text is selected", async () => {
    const { editing, pages, selectHello, highlightButton, user } = await setup();
    expect(highlightButton()).toBeDisabled();
    expect(highlightButton()).toHaveAccessibleName(strings.toolbar.highlightNeedsText);

    selectHello();
    expect(highlightButton()).toBeEnabled();
    expect(highlightButton()).toHaveAccessibleName(strings.toolbar.highlight(t.colors.yellow));
    await user.click(highlightButton());
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    expect(lastEdit(editing)).toMatchObject({ kind: "addHighlight", color: "yellow" });

    fireEvent.contextMenu(pages, at(0, 80, 106));
    await user.click(await screen.findByRole("menuitem", { name: t.highlight }));
    fireEvent.click(await screen.findByRole("menuitem", { name: t.colors.pink }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(2));
    expect(highlightButton()).toHaveAccessibleName(strings.toolbar.highlight(t.colors.pink));
  });

  it("is not offered when the author forbids annotating, or when nothing is selected (MVP-19)", async () => {
    const { editing, pages, selectHello, highlightButton, user } = await setup({ ...ALL_PERMISSIONS, annotate: false });
    selectHello();
    expect(highlightButton()).toBeDisabled();
    fireEvent.contextMenu(pages, at(0, 80, 106));
    expect(await screen.findByRole("menuitem", { name: t.highlight })).toHaveAttribute("aria-disabled", "true");
    const addNote = screen.getByRole("menuitem", { name: new RegExp(t.addNote.replace("…", "")) });
    expect(addNote).toHaveAttribute("aria-disabled", "true");
    expect(addNote).toHaveTextContent(strings.permissions.notAllowed);
    await user.keyboard("{Escape}");
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });

  it("says when too much is selected to keep in one mark, and when saving first would help", async () => {
    const { editing, selectHello, highlightButton, user } = await setup();
    selectHello();
    editing.applyEdit.mockRejectedValueOnce({ code: "limitExceeded", message: "" });
    await user.click(highlightButton());
    expect(await screen.findByText(strings.pages.saveFirst)).toBeInTheDocument();
    editing.applyEdit.mockRejectedValueOnce({ code: "internal", message: "" });
    await user.click(highlightButton());
    expect(await screen.findByText(t.failed)).toBeInTheDocument();
  });
});

describe("notes (B2-07)", () => {
  it("are put where the context menu was opened, with what the user types", async () => {
    const { editing, pages, user } = await setup();
    fireEvent.contextMenu(pages, at(0, 100, 200));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(t.addNote.replace("…", "")) }));
    const dialog = await screen.findByRole("dialog", { name: t.noteDialog.addTitle });
    await user.type(within(dialog).getByLabelText(t.noteDialog.label), "  Check this{Enter}  twice  ");
    await user.click(within(dialog).getByRole("button", { name: t.noteDialog.save }));

    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(1));
    const edit = lastEdit(editing);
    expect(edit).toMatchObject({ kind: "addNote", page: 0, text: "Check this\n  twice" });
    expect(edit.kind === "addNote" && edit.at.x).toBeCloseTo(100, 0);
    expect(edit.kind === "addNote" && edit.at.y).toBeCloseTo(200, 0);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.noteDialog.addTitle })).toBeNull());
  });

  it("need some text, and not too much, and can be given up", async () => {
    const { editing, pages, user } = await setup();
    fireEvent.contextMenu(pages, at(0, 100, 200));
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(t.addNote.replace("…", "")) }));
    const dialog = await screen.findByRole("dialog", { name: t.noteDialog.addTitle });
    const field = within(dialog).getByLabelText(t.noteDialog.label);
    await user.type(field, "   ");
    await user.click(within(dialog).getByRole("button", { name: t.noteDialog.save }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.noteDialog.empty);

    fireEvent.change(field, { target: { value: "字".repeat(2000) } });
    await user.click(within(dialog).getByRole("button", { name: t.noteDialog.save }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(t.noteDialog.tooLong);
    expect(editing.applyEdit).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: t.noteDialog.cancel }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.noteDialog.addTitle })).toBeNull());
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });

  it("are not put outside a page", async () => {
    const { editing, user } = await setup();
    // Far to the right of every page.
    fireEvent.contextMenu(screen.getByRole("img", { name: "第 1 頁" }).parentElement!, { clientX: 990, clientY: 100 });
    await user.click(await screen.findByRole("menuitem", { name: new RegExp(t.addNote.replace("…", "")) }));
    expect(await screen.findByText(t.notOnPage)).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });
});

describe("the annotations a page has (B2-07)", () => {
  it("are chosen with the keyboard, and changed or removed from their toolbar", async () => {
    const { editing, user } = await setup();
    const mark = await screen.findByRole("button", { name: annotationLabel(highlight) });
    act(() => mark.focus());
    const toolbar = await screen.findByRole("toolbar", { name: annotationLabel(highlight) });
    expect(within(toolbar).getByRole("button", { name: t.colors.yellow })).toHaveAttribute("aria-pressed", "true");
    // The toolbar is the next stop of Tab, not the outlines of the rest of the page.
    await user.tab();
    expect(within(toolbar).getByRole("button", { name: t.colors.yellow })).toHaveFocus();
    await user.click(within(toolbar).getByRole("button", { name: t.colors.blue }));
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, { kind: "setHighlightColor", page: 0, annotation: 4, color: "blue" });
    await user.click(within(toolbar).getByRole("button", { name: t.delete }));
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, { kind: "deleteAnnotation", page: 0, annotation: 4 });
    // The Delete key does the same from anywhere in it; Escape lets go, and the view has the focus.
    await user.keyboard("{Delete}");
    expect(editing.applyEdit).toHaveBeenCalledTimes(3);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("toolbar", { name: annotationLabel(highlight) })).toBeNull();
    expect(screen.getByRole("main")).toHaveFocus();
  });

  it("are chosen with a click, anywhere on them, and let go by a click elsewhere", async () => {
    const { pages } = await setup();
    await screen.findByRole("button", { name: annotationLabel(note) });
    fireEvent.mouseDown(pages, { button: 0, detail: 1, ...at(0, 310, 310) });
    fireEvent.click(pages, at(0, 310, 310));
    expect(await screen.findByRole("toolbar", { name: annotationLabel(note) })).toBeInTheDocument();
    fireEvent.mouseDown(pages, { button: 0, detail: 1, ...at(0, 500, 500) });
    fireEvent.click(pages, at(0, 500, 500));
    await waitFor(() => expect(screen.queryByRole("toolbar", { name: annotationLabel(note) })).toBeNull());
  });

  it("have notes whose text is changed in a dialog", async () => {
    const { editing, user } = await setup();
    act(() => screen.getByRole("button", { name: annotationLabel(note) }).focus());
    const toolbar = await screen.findByRole("toolbar", { name: annotationLabel(note) });
    expect(toolbar).toHaveTextContent("Existing note");
    await user.click(within(toolbar).getByRole("button", { name: t.editNote }));
    const dialog = await screen.findByRole("dialog", { name: t.noteDialog.editTitle });
    const field = within(dialog).getByLabelText(t.noteDialog.label);
    expect(field).toHaveValue("Existing note");
    await user.clear(field);
    await user.type(field, "Changed");
    await user.click(within(dialog).getByRole("button", { name: t.noteDialog.save }));
    await waitFor(() =>
      expect(editing.applyEdit).toHaveBeenCalledWith(5, { kind: "setNoteText", page: 0, annotation: 5, text: "Changed" }),
    );
  });

  it("can only be looked at when the author forbids changing them", async () => {
    const { editing, user } = await setup({ ...ALL_PERMISSIONS, annotate: false });
    act(() => screen.getByRole("button", { name: annotationLabel(highlight) }).focus());
    const toolbar = await screen.findByRole("toolbar", { name: annotationLabel(highlight) });
    for (const button of within(toolbar).getAllByRole("button")) expect(button).toBeDisabled();
    await user.keyboard("{Delete}");
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });
});

describe("PageAnnotations", () => {
  const page = { widthPt: 612, heightPt: 792 };
  const box = { top: 16, width: 816, height: 1056 };

  function mount(chosen: number | null, list: PageAnnotation[]) {
    const source = createAnnotationSource({ getPageAnnotations: () => Promise.resolve(list) });
    const onChoose = vi.fn();
    render(
      <PageAnnotations
        source={source}
        doc={1}
        index={0}
        page={page}
        rotation={0}
        box={box}
        left={0}
        delayMs={0}
        chosen={chosen}
        onChoose={onChoose}
        onRelease={vi.fn()}
      />,
    );
    return { onChoose };
  }

  it("lets go of an annotation an edit took away", async () => {
    const { onChoose } = mount(99, [highlight]);
    await screen.findByRole("button", { name: annotationLabel(highlight) });
    await waitFor(() => expect(onChoose).toHaveBeenCalledWith(null));
  });

  it("keeps the one that is still there, and draws nothing for a page without annotations", async () => {
    const { onChoose } = mount(4, [highlight]);
    await screen.findByRole("button", { name: annotationLabel(highlight) });
    expect(onChoose).not.toHaveBeenCalled();
    expect(screen.getByRole("toolbar", { name: annotationLabel(highlight) })).toBeInTheDocument();
  });

  it("shows nothing for a page without annotations", async () => {
    mount(null, []);
    await act(async () => {});
    expect(screen.queryByRole("button")).toBeNull();
  });
});
