// Turning a picture stamp (B3-01): the buttons, the keys and the handle of a chosen picture send
// the angle it is to have; a stamp that is no picture of the user's own, and a document whose
// author forbids annotating, have none of them.
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { type AnnotationsApi } from "@/features/annotations/source";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";
import type { Edit, PageAnnotation, PictureView } from "@/ipc/generated/contract";

const t = strings.annotations;

/** A picture 128 by 64 points, turned `angle` degrees, in the box that holds it. */
function picture(angle: number): PageAnnotation {
  const view: PictureView = { angle, width: 128, height: 64 };
  return {
    id: 11,
    kind: "stamp",
    rect: { x0: 100, y0: 300, x1: 228, y1: 364 },
    color: null,
    text: null,
    picture: view,
  };
}

const standard: PageAnnotation = {
  id: 12,
  kind: "stamp",
  rect: { x0: 300, y0: 500, x1: 450, y1: 540 },
  color: null,
  text: null,
  picture: null,
};

async function setup(onPage: PageAnnotation[], permissions: DocumentPermissions = ALL_PERMISSIONS) {
  const editing = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
  const annotationsApi = {
    getPageAnnotations: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? onPage : [])),
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
  return { editing, user };
}

/** Chooses the stamp by focusing its outline, as the keyboard does. */
const choose = async () => {
  const outline = await screen.findByRole("button", { name: t.kind.stamp });
  act(() => outline.focus());
  return outline;
};

const lastEdit = (editing: { applyEdit: ReturnType<typeof vi.fn> }): Edit => editing.applyEdit.mock.calls.at(-1)![1] as Edit;
const handle = () => document.querySelector<HTMLElement>("[data-turn-handle]");
const outline = () => document.querySelector<SVGPolygonElement>("[data-turn-outline]");

/** The box the outline's corners lie in, as the CSS pixels of the page. */
function outlineBox() {
  const points = outline()!
    .getAttribute("points")!
    .split(" ")
    .map((pair) => pair.split(",").map(Number) as [number, number]);
  const xs = points.map(([x]) => x);
  const ys = points.map(([, y]) => y);
  return { width: Math.max(...xs) - Math.min(...xs), height: Math.max(...ys) - Math.min(...ys) };
}

describe("turning a picture with the buttons and the keys (B3-01)", () => {
  it("sends the angle a step to either side of the one it has, and says how in the outline's description", async () => {
    const { editing, user } = await setup([picture(0)]);
    const stamp = await choose();
    expect(stamp).toHaveAccessibleDescription(t.moveTurnHint);
    await user.click(screen.getByRole("button", { name: t.turnRight }));
    expect(lastEdit(editing)).toEqual({ kind: "setPictureAngle", page: 0, annotation: 11, degrees: 15 });
    await user.click(screen.getByRole("button", { name: t.turnLeft }));
    expect(lastEdit(editing)).toEqual({ kind: "setPictureAngle", page: 0, annotation: 11, degrees: -15 });
  });

  it("goes round: from 170 a step to the right is -175, not 185", async () => {
    const { editing, user } = await setup([picture(170)]);
    await choose();
    await user.click(screen.getByRole("button", { name: t.turnRight }));
    expect(lastEdit(editing)).toMatchObject({ kind: "setPictureAngle", degrees: -175 });
  });

  it("turns a step with the bracket keys, and not again for a key that is held", async () => {
    const { editing } = await setup([picture(30)]);
    const stamp = await choose();
    fireEvent.keyDown(stamp, { key: "]" });
    expect(lastEdit(editing)).toMatchObject({ kind: "setPictureAngle", annotation: 11, degrees: 45 });
    fireEvent.keyDown(stamp, { key: "[" });
    expect(lastEdit(editing)).toMatchObject({ degrees: 15 });
    const sent = editing.applyEdit.mock.calls.length;
    fireEvent.keyDown(stamp, { key: "]", repeat: true });
    expect(editing.applyEdit).toHaveBeenCalledTimes(sent);
  });

  it("is for a picture of the user's own: a standard stamp has no such buttons, keys or handle", async () => {
    const { editing } = await setup([standard]);
    const stamp = await choose();
    expect(screen.queryByRole("button", { name: t.turnRight })).toBeNull();
    expect(screen.queryByRole("button", { name: t.turnLeft })).toBeNull();
    expect(stamp).toHaveAccessibleDescription(t.moveHint);
    fireEvent.keyDown(stamp, { key: "]" });
    expect(editing.applyEdit).not.toHaveBeenCalled();
    expect(handle()).toBeNull();
    expect(outline()).toBeNull();
  });

  it("can only be looked at when the author forbids annotating", async () => {
    const { editing } = await setup([picture(0)], { ...ALL_PERMISSIONS, annotate: false });
    const stamp = await choose();
    expect(screen.getByRole("button", { name: t.turnRight })).toBeDisabled();
    expect(screen.getByRole("button", { name: t.turnLeft })).toBeDisabled();
    fireEvent.keyDown(stamp, { key: "]" });
    expect(editing.applyEdit).not.toHaveBeenCalled();
    expect(handle()).toBeNull();
  });
});

describe("turning a picture with the handle (B3-01)", () => {
  // 128 x 64 points at 100% are 170.7 x 85.3 CSS pixels: the handle is 24 above the top edge, which
  // is 42.7 above the centre.
  const REACH = (64 * 4) / 3 / 2 + 24;
  const pointer = (x: number, y: number, shiftKey = false) => ({ pointerId: 2, button: 0, clientX: x, clientY: y, shiftKey });

  it("shows the picture's outline as it is, and the handle above it", async () => {
    await setup([picture(0)]);
    await choose();
    expect(handle()).not.toBeNull();
    const box = outlineBox();
    expect(box.width).toBeCloseTo((128 * 4) / 3);
    expect(box.height).toBeCloseTo((64 * 4) / 3);
    // Turned a quarter, the outline is as tall as the picture is wide.
  });

  it("shows a picture that is turned as it is turned", async () => {
    await setup([{ ...picture(90), rect: { x0: 132, y0: 268, x1: 196, y1: 396 } }]);
    await choose();
    const box = outlineBox();
    expect(box.width).toBeCloseTo((64 * 4) / 3);
    expect(box.height).toBeCloseTo((128 * 4) / 3);
  });

  it("follows the pointer while it is down, and sends the angle it was let go at", async () => {
    const { editing } = await setup([picture(0)]);
    await choose();
    fireEvent.pointerDown(handle()!, pointer(500, 300));
    // The pointer went to the right of the centre, level with it: a quarter turn.
    fireEvent.pointerMove(handle()!, pointer(500 + REACH, 300 + REACH));
    expect(editing.applyEdit).not.toHaveBeenCalled();
    const box = outlineBox();
    expect(box.width).toBeCloseTo((64 * 4) / 3, 0);
    expect(box.height).toBeCloseTo((128 * 4) / 3, 0);
    fireEvent.pointerUp(handle()!, pointer(500 + REACH, 300 + REACH));
    expect(lastEdit(editing)).toEqual({ kind: "setPictureAngle", page: 0, annotation: 11, degrees: 90 });
  });

  it("goes by steps of 15 degrees with Shift", async () => {
    const { editing } = await setup([picture(0)]);
    await choose();
    fireEvent.pointerDown(handle()!, pointer(500, 300));
    // 22 degrees: the vector from the centre is REACH long, 22 degrees from straight up.
    const radians = (22 * Math.PI) / 180;
    const [x, y] = [REACH * Math.sin(radians), -REACH * Math.cos(radians)];
    fireEvent.pointerMove(handle()!, pointer(500 + x, 300 + y + REACH, true));
    fireEvent.pointerUp(handle()!, pointer(500 + x, 300 + y + REACH, true));
    expect(lastEdit(editing)).toMatchObject({ kind: "setPictureAngle", degrees: 15 });
  });

  it("sends nothing for a press that did not turn it, nor for one that was cancelled", async () => {
    const { editing } = await setup([picture(0)]);
    await choose();
    fireEvent.pointerDown(handle()!, pointer(500, 300));
    fireEvent.pointerUp(handle()!, pointer(500, 300));
    expect(editing.applyEdit).not.toHaveBeenCalled();
    fireEvent.pointerDown(handle()!, pointer(500, 300));
    fireEvent.pointerMove(handle()!, pointer(500 + REACH, 300 + REACH));
    fireEvent.pointerCancel(handle()!, { pointerId: 2 });
    fireEvent.pointerUp(handle()!, pointer(500 + REACH, 300 + REACH));
    expect(editing.applyEdit).not.toHaveBeenCalled();
    // Back where it was: the outline is upright again.
    expect(outlineBox().width).toBeCloseTo((128 * 4) / 3);
  });
});
