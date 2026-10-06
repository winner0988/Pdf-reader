import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { FormsApi } from "@/features/forms/source";
import { ALL_PERMISSIONS } from "@/features/permissions/permissions";
import { demoDocument } from "@/features/shell/demo";
import type { DocumentPermissions, ShellDocument } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { SavingApi } from "@/features/saving/api";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";
import type { FormField } from "@/ipc/generated/contract";

const t = strings.forms;

const place = (y: number) => ({ x0: 72, y0: y, x1: 300, y1: y + 24 });
const field = (id: number, label: string, extra: Partial<FormField> = {}): FormField => ({
  id,
  group: id,
  kind: "text",
  rect: place(100 + id * 30),
  label,
  value: "",
  onValue: null,
  options: [],
  readOnly: false,
  required: false,
  multiline: false,
  password: false,
  editable: false,
  multiSelect: false,
  maxLen: null,
  hasScript: false,
  ...extra,
});

/** The page's fields, in the order the worker lists them (the page's own order). */
const FIELDS: FormField[] = [
  field(6, "Your name", { value: "Jane" }),
  field(7, "Notes", { multiline: true }),
  field(8, "Code", { maxLen: 5 }),
  field(11, "I agree", { kind: "checkbox", value: "Off", onValue: "Yes" }),
  field(14, "Size", { kind: "radio", group: 21, value: "Off", onValue: "Small" }),
  field(17, "Size", { kind: "radio", group: 21, value: "Off", onValue: "Medium" }),
  field(
    22,
    "Country",
    {
      kind: "combo",
      value: "TW",
      options: [
        { value: "TW", label: "Taiwan" },
        { value: "JP", label: "Japan" },
      ],
    },
  ),
  field(23, "Fruit", {
    kind: "list",
    value: "banana",
    options: [
      { value: "apple", label: "apple" },
      { value: "banana", label: "banana" },
    ],
  }),
  field(24, "Locked", { value: "fixed", readOnly: true }),
  field(25, "Required", { required: true }),
  field(26, "Amount", { hasScript: true }),
  field(27, "Secret", { password: true }),
];

type Options = {
  permissions?: DocumentPermissions;
  document?: Partial<ShellDocument>;
  /** The main process moves on with every edit, as it does: a new document id, with changes. */
  live?: boolean;
  /** The new document is announced this long after the edit is answered (a big document is). */
  announceAfterMs?: number;
};

async function setup({
  permissions = ALL_PERMISSIONS,
  document = {},
  live = false,
  announceAfterMs = 0,
}: Options = {}) {
  /** The document as the main process has it, which the page shows a moment later. */
  const main = { doc: 5, unsaved: false };
  const editing = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => {
      if (live) {
        const move = () => {
          main.doc += 1;
          main.unsaved = true;
        };
        if (announceAfterMs > 0) setTimeout(move, announceAfterMs);
        else move();
      }
      return Promise.resolve();
    }),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
  const saving = {
    save: vi.fn<SavingApi["save"]>(() => Promise.resolve({ incremental: false })),
    saveAs: vi.fn<SavingApi["saveAs"]>(() => Promise.resolve(null)),
    closeWindow: vi.fn<SavingApi["closeWindow"]>(() => Promise.resolve()),
    privacyExport: vi.fn<SavingApi["privacyExport"]>(() => Promise.resolve(true)),
  } satisfies SavingApi;
  const formsApi = {
    getPageFields: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? FIELDS : [])),
  } satisfies FormsApi;
  const shell = (doc: number, extra: Partial<ShellDocument> = {}) => (
    <ReaderShell
      state={{ kind: "open", document: { ...demoDocument, doc, permissions, hasForm: true, ...document, ...extra } }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      editingApi={editing}
      savingApi={saving}
      formsApi={formsApi}
      latest={live ? () => ({ ...main }) : undefined}
    />
  );
  const view = render(shell(5));
  const user = userEvent.setup({ pointerEventsCheck: PointerEventsCheckLevel.Never });
  const canvas = screen.getByRole("main");
  Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
  Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 800 });
  act(() => canvas.dispatchEvent(new Event("scroll")));
  await waitFor(() => expect(formsApi.getPageFields).toHaveBeenCalledWith(5, 0));
  await screen.findByRole("textbox", { name: "Your name" });
  return {
    editing,
    saving,
    formsApi,
    main,
    user,
    show: (doc: number, extra?: Partial<ShellDocument>) => view.rerender(shell(doc, extra)),
    box: (label: string) => screen.getByLabelText(label),
  };
}

describe("filling in a form (B2-09)", () => {
  it("shows the fields where the page lists them, in its order, with what they are", async () => {
    const { box } = await setup();
    const order = Array.from(document.querySelectorAll("[data-page-fields] [data-field]")).map((element) =>
      element.getAttribute("aria-label"),
    );
    expect(order).toEqual(
      FIELDS.map((each) => (each.kind === "radio" ? t.choice(each.label!, each.onValue!) : each.label)),
    );
    expect(box("Your name")).toHaveValue("Jane");
    expect(box("Notes").tagName).toBe("TEXTAREA");
    expect(box("Code")).toHaveAttribute("maxlength", "5");
    expect(box("Secret")).toHaveAttribute("type", "password");
    expect(box("Locked")).toHaveAttribute("readonly");
    expect(box("Required")).toHaveAttribute("aria-required", "true");
    expect(box("Required")).toHaveAttribute("aria-invalid", "true");
    expect(box("I agree")).not.toBeChecked();
    expect(box("Country")).toHaveValue("TW");
    expect(within(box("Country")).getByRole("option", { name: "Taiwan" })).toBeInTheDocument();
    expect(box("Fruit")).toHaveValue("banana");
    // The buttons of a group are one group for the keyboard.
    expect(box(t.choice("Size", "Small"))).toHaveAttribute("name", "field-21");
    expect(box(t.choice("Size", "Medium"))).toHaveAttribute("name", "field-21");
  });

  it("sends a text field when the user leaves it, once, and keeps what was typed", async () => {
    const { editing, user, box } = await setup();
    const name = box("Your name");
    await user.clear(name);
    await user.type(name, "林 小明");
    expect(editing.applyEdit).not.toHaveBeenCalled();
    await user.tab();
    expect(editing.applyEdit).toHaveBeenCalledTimes(1);
    expect(editing.applyEdit).toHaveBeenCalledWith(5, {
      kind: "setFieldValue",
      page: 0,
      field: 6,
      value: "林 小明",
    });
    // Until the document has it, the box shows what the user made of it.
    expect(name).toHaveValue("林 小明");
    // Nothing changed: nothing is sent.
    await user.click(box("Notes"));
    await user.tab();
    expect(editing.applyEdit).toHaveBeenCalledTimes(1);
  });

  it("sends a one-line field on Enter, and gives up a draft on Escape", async () => {
    const { editing, user, box } = await setup();
    const name = box("Your name");
    await user.type(name, " Q.{Enter}");
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, expect.objectContaining({ field: 6, value: "Jane Q." }));
    await user.click(box("Code"));
    await user.type(box("Code"), "AB{Escape}");
    expect(editing.applyEdit).toHaveBeenCalledTimes(1);
    expect(box("Code")).toHaveValue("");
    // In several lines Enter is a line break; Ctrl+Enter is done.
    const notes = box("Notes");
    await user.click(notes);
    await user.type(notes, "one{Enter}two");
    expect(editing.applyEdit).toHaveBeenCalledTimes(1);
    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, expect.objectContaining({ field: 7, value: "one\ntwo" }));
  });

  it("sends a check box and a radio button at once, each way", async () => {
    const { editing, user, box } = await setup();
    await user.click(box("I agree"));
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, { kind: "setFieldValue", page: 0, field: 11, value: "Yes" });
    expect(box("I agree")).toBeChecked();
    await user.click(box("I agree"));
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, { kind: "setFieldValue", page: 0, field: 11, value: "Off" });
    await user.click(box(t.choice("Size", "Medium")));
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, { kind: "setFieldValue", page: 0, field: 17, value: "Medium" });
  });

  it("sends a drop-down list and a list box when the user picks", async () => {
    const { editing, user, box } = await setup();
    await user.selectOptions(box("Country"), "JP");
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, { kind: "setFieldValue", page: 0, field: 22, value: "JP" });
    await user.selectOptions(box("Fruit"), "apple");
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, { kind: "setFieldValue", page: 0, field: 23, value: "apple" });
  });

  it("takes back what could not be made, and says so", async () => {
    const { editing, user, box } = await setup();
    editing.applyEdit.mockRejectedValueOnce({ code: "invalidArgument", message: "" });
    await user.type(box("Your name"), "x");
    await user.tab();
    expect(await screen.findByText(t.failed)).toBeInTheDocument();
    await waitFor(() => expect(box("Your name")).toHaveValue("Jane"));
    editing.applyEdit.mockRejectedValueOnce({ code: "limitExceeded", message: "" });
    await user.click(box("I agree"));
    expect(await screen.findByText(strings.pages.saveFirst)).toBeInTheDocument();
    await waitFor(() => expect(box("I agree")).not.toBeChecked());
  });

  it("keeps the focus where it is when the document answers with its new fields", async () => {
    const { user, show, box, formsApi } = await setup();
    await user.type(box("Your name"), "!");
    await user.tab();
    // The next field in the page's order has the focus.
    const notes = box("Notes");
    expect(notes).toHaveFocus();
    // The document is a new one now, with the value in it; the boxes are the same ones.
    formsApi.getPageFields.mockImplementation((_doc, page) =>
      Promise.resolve(page === 0 ? FIELDS.map((each) => (each.id === 6 ? { ...each, value: "Jane!" } : each)) : []),
    );
    show(6);
    await waitFor(() => expect(box("Your name")).toHaveValue("Jane!"));
    expect(box("Notes")).toBe(notes);
    expect(notes).toHaveFocus();
  });

  it("says that the scripts of a field are not run, when the user enters it", async () => {
    const { box, user } = await setup();
    await user.click(box("Amount"));
    expect(await screen.findByText(t.scriptNotRun)).toBeInTheDocument();
  });

  it("can only be looked at when the author forbids filling in the form (MVP-19)", async () => {
    const { editing, box, user } = await setup({ permissions: { ...ALL_PERMISSIONS, fillForms: false } });
    // A text box can still be read and selected; the rest cannot be used.
    expect(box("Your name")).toHaveAttribute("readonly");
    expect(box("Your name")).toHaveValue("Jane");
    for (const label of ["I agree", "Country"]) expect(box(label)).toBeDisabled();
    for (const label of ["Your name", "I agree", "Country"]) {
      expect(box(label)).toHaveAttribute("title", t.notAllowed);
    }
    await user.click(box("I agree"));
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });

  it("leaves the keys of a list of options to the list", async () => {
    const { box, user } = await setup();
    await user.click(box("Fruit"));
    await user.keyboard("{Home}");
    // Home chose the first option; the page did not go to its first page.
    expect(screen.getByRole("contentinfo")).toHaveTextContent(strings.statusBar.pageStatus(1, 12, strings.toolbar.fitWidth));
  });
});

describe("flattening the form (B2-09)", () => {
  const openMenu = async (user: ReturnType<typeof userEvent.setup>) => {
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    return screen.findByRole("menuitem", { name: new RegExp(t.flatten.replace("…", "")) });
  };

  it("is offered for a document with a form, asked about, and followed by where to save it", async () => {
    const { editing, saving, user } = await setup({ live: true });
    await user.click(await openMenu(user));
    const dialog = await screen.findByRole("dialog", { name: t.flattenDialog.title });
    await user.click(within(dialog).getByRole("button", { name: t.flattenDialog.confirm }));
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledWith(5, { kind: "flattenForm" }));
    // Where to save is asked at once, for the document that edit made (the flattened one).
    await waitFor(() => expect(saving.saveAs).toHaveBeenCalledWith(6));
    expect(saving.saveAs).toHaveBeenCalledTimes(1);
  });

  it("can be given up", async () => {
    const { editing, user } = await setup();
    await user.click(await openMenu(user));
    const dialog = await screen.findByRole("dialog", { name: t.flattenDialog.title });
    await user.click(within(dialog).getByRole("button", { name: t.flattenDialog.cancel }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: t.flattenDialog.title })).toBeNull());
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });

  it("says so when it cannot be done, and asks for no file", async () => {
    const { editing, saving, user } = await setup();
    editing.applyEdit.mockRejectedValueOnce({ code: "invalidArgument", message: "" });
    await user.click(await openMenu(user));
    await user.click(within(await screen.findByRole("dialog", { name: t.flattenDialog.title })).getByRole("button", { name: t.flattenDialog.confirm }));
    expect(await screen.findByText(t.flattenFailed)).toBeInTheDocument();
    expect(saving.saveAs).not.toHaveBeenCalled();
  });

  it("is not offered without a form, and says why when the author forbids it", async () => {
    const without = await setup({ document: { hasForm: false } });
    await without.user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    expect(screen.queryByRole("menuitem", { name: new RegExp(t.flatten.replace("…", "")) })).toBeNull();
  });

  it("says that the author does not allow it, and does nothing (MVP-19)", async () => {
    const { editing, user } = await setup({ permissions: { ...ALL_PERMISSIONS, fillForms: false } });
    const item = await openMenu(user);
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveTextContent(strings.permissions.notAllowed);
    await user.click(item);
    expect(screen.queryByRole("dialog", { name: t.flattenDialog.title })).toBeNull();
    expect(editing.applyEdit).not.toHaveBeenCalled();
  });
});

describe("sending values one after another (B2-09)", () => {
  const nameEdit = { kind: "setFieldValue", page: 0, field: 6, value: "Jane!" };
  const notesEdit = { kind: "setFieldValue", page: 0, field: 7, value: "x" };

  it("sends each value with the document the one before made, without waiting for the page to show it", async () => {
    const { editing, user, box } = await setup({ live: true });
    await user.type(box("Your name"), "!");
    await user.tab();
    await user.type(box("Notes"), "x");
    await user.tab();
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(2));
    expect(editing.applyEdit).toHaveBeenNthCalledWith(1, 5, nameEdit);
    expect(editing.applyEdit).toHaveBeenNthCalledWith(2, 6, notesEdit);
  });

  it("sends the next value only once the one before was answered", async () => {
    const { editing, main, user, box } = await setup({ live: true });
    let answer = () => {};
    editing.applyEdit.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          answer = () => {
            main.doc += 1;
            main.unsaved = true;
            resolve();
          };
        }),
    );
    await user.type(box("Your name"), "!");
    await user.tab();
    await user.type(box("Notes"), "x");
    await user.tab();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(editing.applyEdit).toHaveBeenCalledTimes(1);
    answer();
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(2));
    expect(editing.applyEdit).toHaveBeenLastCalledWith(6, notesEdit);
  });

  it("waits for the document an edit made to be announced, which can come after the answer", async () => {
    const { editing, user, box } = await setup({ live: true, announceAfterMs: 100 });
    await user.selectOptions(box("Country"), "JP");
    await user.selectOptions(box("Fruit"), "apple");
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(2));
    expect(editing.applyEdit).toHaveBeenNthCalledWith(1, 5, { kind: "setFieldValue", page: 0, field: 22, value: "JP" });
    expect(editing.applyEdit).toHaveBeenNthCalledWith(2, 6, { kind: "setFieldValue", page: 0, field: 23, value: "apple" });
  });

  it("saves the document that was announced late, not the one the page still shows", async () => {
    const { saving, user, box } = await setup({ live: true, announceAfterMs: 100 });
    await user.type(box("Your name"), "!{Enter}");
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(saving.save).toHaveBeenCalledWith(6));
  });

  it("goes on with the next value after one that could not be made", async () => {
    const { editing, user, box } = await setup({ live: true });
    editing.applyEdit.mockRejectedValueOnce({ code: "invalidArgument", message: "" });
    await user.type(box("Your name"), "!");
    await user.tab();
    await user.type(box("Notes"), "x");
    await user.tab();
    await waitFor(() => expect(editing.applyEdit).toHaveBeenCalledTimes(2));
    // The first one changed nothing, so the document is still the first one.
    expect(editing.applyEdit).toHaveBeenLastCalledWith(5, notesEdit);
  });

  it("sends the value being typed before saving, and saves the document it made (Ctrl+S)", async () => {
    const { editing, saving, user, box } = await setup({ live: true });
    await user.type(box("Your name"), "!");
    // Still in the box: nothing was sent yet, and the document has no changes.
    expect(editing.applyEdit).not.toHaveBeenCalled();
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(saving.save).toHaveBeenCalledWith(6));
    expect(editing.applyEdit).toHaveBeenCalledWith(5, nameEdit);
    expect(saving.save).toHaveBeenCalledTimes(1);
  });

  it("sends the value being typed before asking where to save a copy (Ctrl+Shift+S)", async () => {
    const { editing, saving, user, box } = await setup({ live: true });
    await user.type(box("Your name"), "!");
    await user.keyboard("{Control>}{Shift>}s{/Shift}{/Control}");
    await waitFor(() => expect(saving.saveAs).toHaveBeenCalledWith(6));
    expect(editing.applyEdit).toHaveBeenCalledWith(5, nameEdit);
  });

  it("saves the document a value in flight made, not the one the page still shows", async () => {
    const { saving, user, box } = await setup({ live: true });
    // Left with Enter: the value is on its way when the key for saving comes.
    await user.type(box("Your name"), "!{Enter}");
    await user.keyboard("{Control>}s{/Control}");
    await waitFor(() => expect(saving.save).toHaveBeenCalledWith(6));
  });

  it("does not save a document without changes", async () => {
    const { editing, saving, user, box } = await setup({ live: true });
    await user.click(box("Your name"));
    await user.keyboard("{Control>}s{/Control}");
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    expect(editing.applyEdit).not.toHaveBeenCalled();
    expect(saving.save).not.toHaveBeenCalled();
  });
});
