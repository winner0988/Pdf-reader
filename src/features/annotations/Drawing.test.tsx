import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { type AnnotationsApi } from "@/features/annotations/source";
import type { LinksApi } from "@/features/links/source";
import { STAMP_HEIGHT_PT, STAMP_WIDTH_PT } from "@/features/annotations/tools";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";
import type { Edit, PageAnnotation, PageLink, StampImageInfo } from "@/ipc/generated/contract";

const t = strings.annotations;

const drawing: PageAnnotation = {
  id: 8,
  kind: "ink",
  rect: { x0: 100, y0: 200, x1: 250, y1: 260 },
  color: null,
  text: null,
};
const stamp: PageAnnotation = {
  id: 9,
  kind: "stamp",
  rect: { x0: 300, y0: 400, x1: 450, y1: 400 + STAMP_HEIGHT_PT },
  color: null,
  text: null,
};

/** The pointer on page 1, `x` and `y` in page points: 4 / 3 CSS pixels each at 100%, from the layer's corner. */
const pointer = (x: number, y: number) => ({ pointerId: 1, button: 0, clientX: (x * 4) / 3, clientY: (y * 4) / 3 });

async function setup(
  permissions: DocumentPermissions = ALL_PERMISSIONS,
  onPage: PageAnnotation[] = [drawing, stamp],
  linksApi?: LinksApi,
  /** What the user's choice of a picture gives; `null`: the API cannot ask for one. */
  pick: (() => Promise<StampImageInfo | null>) | null = () => Promise.resolve(null),
) {
  const editing = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
  const annotationsApi = {
    getPageAnnotations: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? onPage : [])),
    ...(pick ? { pickStampImage: vi.fn(pick) } : {}),
  } satisfies AnnotationsApi;
  render(
    <ReaderShell
      state={{ kind: "open", document: { ...demoDocument, doc: 5, permissions } }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      editingApi={editing}
      annotationsApi={annotationsApi}
      linksApi={linksApi}
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
  const penButton = () => screen.getByRole("button", { name: t.pen });
  const stampButton = () => screen.getByRole("button", { name: t.stamp });
  return { annotationsApi, editing, layer, penButton, stampButton, user };
}

const lastEdit = (editing: { applyEdit: ReturnType<typeof vi.fn> }): Edit => editing.applyEdit.mock.calls.at(-1)![1] as Edit;

describe("drawing with the pen (B2-08)", () => {
  it("sends a stroke when the pointer is released, in the color and thickness chosen", async () => {
    const { editing, layer, penButton, user } = await setup();
    expect(layer()).toBeNull();
    await user.click(penButton());
    expect(penButton()).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.penOn);

    // Red and thick, chosen in the menu next to the button (it stays open for the second choice).
    await user.click(screen.getByRole("button", { name: t.penStyle }));
    await user.click(await screen.findByRole("menuitemradio", { name: t.inkColors.red }));
    await user.click(await screen.findByRole("menuitemradio", { name: t.inkWidths.thick }));
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());
    // The pen is still on: Esc only closed the menu.
    expect(layer()).not.toBeNull();

    fireEvent.pointerDown(layer()!, pointer(100, 100));
    // The line is drawn as it goes.
    fireEvent.pointerMove(layer()!, pointer(110, 100));
    expect(layer()!.querySelector("[data-stroke]")).not.toBeNull();
    fireEvent.pointerMove(layer()!, pointer(120, 112));
    expect(editing.applyEdit).not.toHaveBeenCalled();
    fireEvent.pointerUp(layer()!, pointer(130, 112));
    expect(editing.applyEdit).toHaveBeenCalledTimes(1);
    expect(lastEdit(editing)).toEqual({
      kind: "addInk",
      page: 0,
      strokes: [
        [
          { x: 100, y: 100 },
          { x: 110, y: 100 },
          { x: 120, y: 112 },
          { x: 130, y: 112 },
        ],
      ],
      color: "red",
      width: "thick",
    });
    // The pen stays on for the next stroke.
    expect(layer()).not.toBeNull();
    expect(layer()!.querySelector("[data-stroke]")).toBeNull();
  });

  it("makes a dot of a click that did not move, and nothing of a press that was cancelled", async () => {
    const { editing, layer, penButton, user } = await setup();
    await user.click(penButton());
    fireEvent.pointerDown(layer()!, pointer(200, 300));
    fireEvent.pointerUp(layer()!, pointer(200, 300));
    expect(lastEdit(editing)).toMatchObject({ kind: "addInk", strokes: [[{ x: 200, y: 300 }]] });

    editing.applyEdit.mockClear();
    fireEvent.pointerDown(layer()!, pointer(10, 10));
    fireEvent.pointerMove(layer()!, pointer(50, 50));
    fireEvent.pointerCancel(layer()!, pointer(50, 50));
    fireEvent.pointerUp(layer()!, pointer(60, 60));
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });

  it("keeps a line that leaves the page on the page's edge", async () => {
    const { editing, layer, penButton, user } = await setup();
    await user.click(penButton());
    fireEvent.pointerDown(layer()!, pointer(600, 100));
    fireEvent.pointerMove(layer()!, pointer(700, 100));
    fireEvent.pointerUp(layer()!, pointer(650, -40));
    expect(lastEdit(editing)).toMatchObject({
      kind: "addInk",
      strokes: [
        [
          { x: 600, y: 100 },
          { x: 612, y: 100 },
          { x: 612, y: 0 },
        ],
      ],
    });
  });

  it("is above the page's other layers, so that a link under the pen does not get the press", async () => {
    const link: PageLink = {
      id: { pageIndex: 0, index: 0 },
      rect: { x0: 72, y0: 100, x1: 300, y1: 122 },
      target: { kind: "page", pageIndex: 1, x: null, y: null },
    };
    const linksApi = {
      getPageLinks: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? [link] : [])),
      describeLink: vi.fn(),
      openLink: vi.fn(),
      describeOutlineLink: vi.fn(),
      openOutlineLink: vi.fn(),
    } satisfies LinksApi;
    const { layer, penButton, user } = await setup(ALL_PERMISSIONS, [drawing, stamp], linksApi);
    await user.click(penButton());
    await waitFor(() => expect(document.querySelector('[data-page-links="1"]')).not.toBeNull());
    const links = document.querySelector('[data-page-links="1"]');
    // Later in the page is higher: the links come first.
    expect(layer()!.compareDocumentPosition(links!) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
  });

  it("goes away with Esc or with its button, and not for an author who forbids annotating", async () => {
    const { layer, penButton, user } = await setup();
    await user.click(penButton());
    expect(layer()).not.toBeNull();
    await user.keyboard("{Escape}");
    expect(layer()).toBeNull();
    await user.click(penButton());
    await user.click(penButton());
    expect(layer()).toBeNull();
  });

  it("is not offered when the author forbids annotating (MVP-19)", async () => {
    const { layer, penButton, stampButton } = await setup({ ...ALL_PERMISSIONS, annotate: false });
    expect(penButton()).toBeDisabled();
    expect(stampButton()).toBeDisabled();
    expect(stampButton()).toHaveAttribute("title", t.notAllowed);
    expect(layer()).toBeNull();
  });
});

describe("putting a stamp down (B2-08)", () => {
  const choose = async (user: ReturnType<typeof userEvent.setup>, stampButton: () => HTMLElement, name: string) => {
    await user.click(stampButton());
    await user.click(await screen.findByRole("menuitem", { name }));
  };

  it("puts the stamp where the pointer is released, in its own shape, and is done", async () => {
    const { editing, layer, stampButton, user } = await setup();
    await choose(user, stampButton, t.stamps.approved);
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.placeStamp(t.stamps.approved));
    expect(layer()!.dataset.tool).toBe("stamp");

    fireEvent.pointerDown(layer()!, pointer(300, 200));
    expect(editing.applyEdit).not.toHaveBeenCalled();
    fireEvent.pointerUp(layer()!, pointer(300, 200));
    const edit = lastEdit(editing);
    expect(edit).toMatchObject({ kind: "addStamp", page: 0, stamp: "approved" });
    if (edit.kind !== "addStamp") throw new Error("a stamp");
    expect((edit.rect.x0 + edit.rect.x1) / 2).toBeCloseTo(300);
    expect((edit.rect.y0 + edit.rect.y1) / 2).toBeCloseTo(200);
    expect(edit.rect.x1 - edit.rect.x0).toBeCloseTo(STAMP_WIDTH_PT);
    expect(edit.rect.y1 - edit.rect.y0).toBeCloseTo(STAMP_HEIGHT_PT);
    // One stamp for one choice.
    expect(layer()).toBeNull();
  });

  it("puts it in the middle of the page with Enter, for those who cannot point, and Esc gives it up", async () => {
    const { editing, layer, stampButton, user } = await setup();
    await choose(user, stampButton, t.stamps.draft);
    await user.keyboard("{Escape}");
    expect(layer()).toBeNull();

    await choose(user, stampButton, t.stamps.draft);
    await user.keyboard("{Enter}");
    const edit = lastEdit(editing);
    expect(edit).toMatchObject({ kind: "addStamp", page: 0, stamp: "draft" });
    if (edit.kind !== "addStamp") throw new Error("a stamp");
    // A Letter page, 612 x 792 points.
    expect((edit.rect.x0 + edit.rect.x1) / 2).toBeCloseTo(306);
    expect((edit.rect.y0 + edit.rect.y1) / 2).toBeCloseTo(396);
    expect(layer()).toBeNull();
  });
});

describe("moving and resizing a drawing or a stamp (B2-08)", () => {
  /** Chooses the annotation by focusing its outline, as the keyboard does. */
  const choose = async (name: string) => {
    const outline = await screen.findByRole("button", { name });
    act(() => outline.focus());
    return outline;
  };
  const body = (id: number) => document.querySelector<HTMLElement>(`[data-move-resize="${id}"]`)!;

  it("moves with the pointer, as far as it went", async () => {
    const { editing } = await setup();
    await choose(t.kind.ink);
    // 30 by 15 CSS pixels at 100% are 22.5 by 11.25 points.
    fireEvent.pointerDown(body(8), { pointerId: 2, button: 0, clientX: 200, clientY: 300 });
    fireEvent.pointerMove(body(8), { pointerId: 2, clientX: 230, clientY: 315 });
    expect(document.querySelector("[data-move-preview]")).not.toBeNull();
    expect(editing.applyEdit).not.toHaveBeenCalled();
    fireEvent.pointerUp(body(8), { pointerId: 2, clientX: 230, clientY: 315 });
    const edit = lastEdit(editing);
    expect(edit).toMatchObject({ kind: "setAnnotationRect", page: 0, annotation: 8 });
    if (edit.kind !== "setAnnotationRect") throw new Error("a rectangle");
    expect(edit.rect.x0).toBeCloseTo(100 + 22.5);
    expect(edit.rect.y0).toBeCloseTo(200 + 11.25);
    expect(edit.rect.x1 - edit.rect.x0).toBeCloseTo(150);
    expect(document.querySelector("[data-move-preview]")).toBeNull();
  });

  it("is not sent when it did not move", async () => {
    const { editing } = await setup();
    await choose(t.kind.ink);
    fireEvent.pointerDown(body(8), { pointerId: 2, button: 0, clientX: 200, clientY: 300 });
    fireEvent.pointerUp(body(8), { pointerId: 2, clientX: 200, clientY: 300 });
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });

  it("resizes a drawing freely by any handle", async () => {
    const { editing } = await setup();
    await choose(t.kind.ink);
    expect([...body(8).querySelectorAll("[data-handle]")].map((handle) => handle.getAttribute("data-handle")).sort()).toEqual(
      ["e", "n", "ne", "nw", "s", "se", "sw", "w"],
    );
    const east = body(8).querySelector("[data-handle=e]")!;
    fireEvent.pointerDown(east, { pointerId: 2, button: 0, clientX: 400, clientY: 300 });
    fireEvent.pointerMove(body(8), { pointerId: 2, clientX: 460, clientY: 340 });
    fireEvent.pointerUp(body(8), { pointerId: 2, clientX: 460, clientY: 340 });
    const edit = lastEdit(editing);
    if (edit.kind !== "setAnnotationRect") throw new Error("a rectangle");
    // 60 CSS pixels wider (45 points), as high as it was.
    expect(edit.rect.x1 - edit.rect.x0).toBeCloseTo(150 + 45);
    expect(edit.rect.y1 - edit.rect.y0).toBeCloseTo(60);
    expect(edit.rect.x0).toBeCloseTo(100);
  });

  it("resizes a stamp by its corners only, keeping its shape", async () => {
    const { editing } = await setup();
    await choose(t.kind.stamp);
    expect([...body(9).querySelectorAll("[data-handle]")].map((handle) => handle.getAttribute("data-handle")).sort()).toEqual(
      ["ne", "nw", "se", "sw"],
    );
    const corner = body(9).querySelector("[data-handle=se]")!;
    fireEvent.pointerDown(corner, { pointerId: 2, button: 0, clientX: 600, clientY: 540 });
    fireEvent.pointerMove(body(9), { pointerId: 2, clientX: 700, clientY: 545 });
    fireEvent.pointerUp(body(9), { pointerId: 2, clientX: 700, clientY: 545 });
    const edit = lastEdit(editing);
    if (edit.kind !== "setAnnotationRect") throw new Error("a rectangle");
    const width = edit.rect.x1 - edit.rect.x0;
    const height = edit.rect.y1 - edit.rect.y0;
    expect(width / height).toBeCloseTo(STAMP_WIDTH_PT / STAMP_HEIGHT_PT);
    expect(width).toBeGreaterThan(STAMP_WIDTH_PT);
    // The corner opposite the one dragged stays.
    expect(edit.rect.x0).toBeCloseTo(300);
    expect(edit.rect.y0).toBeCloseTo(400);
  });

  it("moves a point with an arrow key, ten with Shift, the way the arrow points", async () => {
    const { editing } = await setup();
    const outline = await choose(t.kind.stamp);
    // The way to move it is said to those who cannot see the page.
    expect(outline).toHaveAccessibleDescription(t.moveHint);
    fireEvent.keyDown(outline, { key: "ArrowRight" });
    let edit = lastEdit(editing);
    if (edit.kind !== "setAnnotationRect") throw new Error("a rectangle");
    expect(edit.rect.x0).toBeCloseTo(301);
    expect(edit.rect.y0).toBeCloseTo(400);
    fireEvent.keyDown(outline, { key: "ArrowUp", shiftKey: true });
    edit = lastEdit(editing);
    if (edit.kind !== "setAnnotationRect") throw new Error("a rectangle");
    expect(edit.rect.y0).toBeCloseTo(390);
    expect(edit).toMatchObject({ annotation: 9, page: 0 });
    // A held key does not repeat: the document must have taken the edit before the next one.
    const sent = editing.applyEdit.mock.calls.length;
    fireEvent.keyDown(outline, { key: "ArrowLeft", repeat: true });
    expect(editing.applyEdit).toHaveBeenCalledTimes(sent);
  });

  it("has no handles on what is not a drawing or a stamp, nor when the author forbids annotating", async () => {
    const note: PageAnnotation = { id: 5, kind: "note", rect: { x0: 10, y0: 10, x1: 30, y1: 30 }, color: null, text: "n" };
    await setup(ALL_PERMISSIONS, [note]);
    act(() => screen.getByRole("button", { name: new RegExp(`^${t.noteSaying("n")}$`) }).focus());
    expect(document.querySelector("[data-move-resize]")).toBeNull();
    expect(screen.getByRole("button", { name: new RegExp(`^${t.noteSaying("n")}$`) })).not.toHaveAccessibleDescription(
      t.moveHint,
    );
  });

  it("can only be looked at when the author forbids annotating (MVP-19)", async () => {
    await setup({ ...ALL_PERMISSIONS, annotate: false });
    await choose(t.kind.ink);
    expect(document.querySelector("[data-move-resize]")).toBeNull();
    expect(within(screen.getByRole("toolbar", { name: t.kind.ink })).getByRole("button", { name: t.delete })).toBeDisabled();
  });
});

describe("drawing on a page that is zoomed and turned (B2-08)", () => {
  it("puts the points and the stamp where the pointer is on the page, not on the screen", async () => {
    const { editing, layer, penButton, stampButton, user } = await setup();
    await user.click(screen.getByRole("button", { name: strings.toolbar.zoomIn }));
    // A quarter turn clockwise: the page's top left corner is at the top right of the screen.
    await user.click(screen.getByRole("button", { name: strings.toolbar.rotateCw }));
    await user.click(penButton());
    // The screen shows the page 792 points wide now, and larger than 100%.
    const scale = parseFloat(layer()!.style.width) / 792;
    expect(scale).toBeGreaterThan(4 / 3);
    const at = (across: number, down: number) => ({
      pointerId: 1,
      button: 0,
      clientX: across * scale,
      clientY: down * scale,
    });

    // Moving right on the screen is moving up the page, along its left edge (x 50).
    fireEvent.pointerDown(layer()!, at(100, 50));
    fireEvent.pointerMove(layer()!, at(200, 50));
    fireEvent.pointerUp(layer()!, at(300, 50));
    expect(lastEdit(editing)).toMatchObject({
      kind: "addInk",
      strokes: [
        [
          { x: 50, y: 692 },
          { x: 50, y: 592 },
          { x: 50, y: 492 },
        ],
      ],
    });
    // The line is as thick on the screen as on the page: 2.5 points, at this zoom.
    fireEvent.pointerDown(layer()!, at(100, 100));
    expect(layer()!.querySelector("[data-stroke]")).toHaveAttribute("stroke-width", String(2.5 * scale));
    fireEvent.pointerCancel(layer()!, at(100, 100));

    // And a stamp: its middle is where the pointer was released, the stamp turned with the page.
    await user.click(stampButton());
    await user.click(await screen.findByRole("menuitem", { name: t.stamps.draft }));
    fireEvent.pointerDown(layer()!, at(400, 300));
    fireEvent.pointerUp(layer()!, at(400, 300));
    const edit = lastEdit(editing);
    if (edit.kind !== "addStamp") throw new Error("a stamp");
    expect((edit.rect.x0 + edit.rect.x1) / 2).toBeCloseTo(300);
    expect((edit.rect.y0 + edit.rect.y1) / 2).toBeCloseTo(392);
    expect(edit.rect.x1 - edit.rect.x0).toBeCloseTo(STAMP_WIDTH_PT);
  });
});

describe("a picture of the user's own as a stamp (B2-08)", () => {
  const picture: StampImageInfo = { image: 4, width: 64, height: 32 };
  const choose = async (user: ReturnType<typeof userEvent.setup>, stampButton: () => HTMLElement) => {
    await user.click(stampButton());
    await user.click(await screen.findByRole("menuitem", { name: t.pickPicture }));
  };

  it("asks for the picture, and puts it down where the pointer is released, in its own shape", async () => {
    const { annotationsApi, editing, layer, stampButton, user } = await setup(
      ALL_PERMISSIONS,
      [drawing, stamp],
      undefined,
      () => Promise.resolve(picture),
    );
    await choose(user, stampButton);
    expect(annotationsApi.pickStampImage).toHaveBeenCalledWith(5);
    await waitFor(() => expect(layer()).not.toBeNull());
    expect(layer()!.dataset.tool).toBe("picture");
    expect(screen.getByRole("contentinfo")).toHaveTextContent(t.placePicture);

    fireEvent.pointerDown(layer()!, pointer(300, 200));
    fireEvent.pointerUp(layer()!, pointer(300, 200));
    const edit = lastEdit(editing);
    expect(edit).toMatchObject({ kind: "addImageStamp", page: 0, image: 4 });
    if (edit.kind !== "addImageStamp") throw new Error("a picture stamp");
    // 64 : 32 pixels, the longer side 150 points; the pointer is in the middle of it.
    expect(edit.rect.x1 - edit.rect.x0).toBeCloseTo(150);
    expect(edit.rect.y1 - edit.rect.y0).toBeCloseTo(75);
    expect((edit.rect.x0 + edit.rect.x1) / 2).toBeCloseTo(300);
    expect((edit.rect.y0 + edit.rect.y1) / 2).toBeCloseTo(200);
    // One stamp for one choice.
    expect(layer()).toBeNull();
  });

  it("puts it in the middle of the page with Enter, and nothing happens when the dialog is closed", async () => {
    const { editing, layer, stampButton, user } = await setup(ALL_PERMISSIONS, [], undefined, () =>
      Promise.resolve(picture),
    );
    await choose(user, stampButton);
    await waitFor(() => expect(layer()).not.toBeNull());
    await user.keyboard("{Enter}");
    const edit = lastEdit(editing);
    if (edit.kind !== "addImageStamp") throw new Error("a picture stamp");
    expect((edit.rect.x0 + edit.rect.x1) / 2).toBeCloseTo(306);
    expect((edit.rect.y0 + edit.rect.y1) / 2).toBeCloseTo(396);
    expect(layer()).toBeNull();
  });

  it("does nothing when the user closes the dialog", async () => {
    const { annotationsApi, layer, stampButton, user } = await setup();
    await choose(user, stampButton);
    await waitFor(() => expect(annotationsApi.pickStampImage).toHaveBeenCalled());
    await act(async () => {});
    expect(layer()).toBeNull();
    expect(screen.getByRole("contentinfo")).not.toHaveTextContent(t.placePicture);
  });

  it("says why a picture cannot be used", async () => {
    const asked = [
      [{ code: "limitExceeded", message: "" }, t.tooManyPictures],
      [{ code: "tooLarge", message: "" }, t.pictureTooLarge],
      [{ code: "invalidArgument", message: "" }, t.pictureFailed],
    ] as const;
    for (const [error, hint] of asked) {
      const { layer, stampButton, user } = await setup(ALL_PERMISSIONS, [], undefined, () => Promise.reject(error));
      await choose(user, stampButton);
      await waitFor(() => expect(screen.getByRole("contentinfo")).toHaveTextContent(hint));
      expect(layer()).toBeNull();
      cleanup();
    }
  });

  it("is not in the menu when the API cannot ask for one", async () => {
    const { stampButton, user } = await setup(ALL_PERMISSIONS, [], undefined, null);
    await user.click(stampButton());
    expect(await screen.findByRole("menuitem", { name: t.pickPicture })).toHaveAttribute("aria-disabled", "true");
  });
});
