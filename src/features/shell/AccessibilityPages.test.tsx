import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent, { PointerEventsCheckLevel } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { annotationLabel } from "@/features/annotations/model";
import type { AnnotationsApi } from "@/features/annotations/source";
import type { FormsApi } from "@/features/forms/source";
import type { LinksApi } from "@/features/links/source";
import type { OcrApi } from "@/features/ocr/api";
import type { RecentApi } from "@/features/recent/api";
import { DEFAULT_SETTINGS, type SettingsApi } from "@/features/settings/api";
import { SettingsProvider } from "@/features/settings/SettingsProvider";
import type { UpdatesApi } from "@/features/settings/updates";
import { demoDocument } from "@/features/shell/demo";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";
import type { SignatureView } from "@/features/signatures/useSignatures";
import type { FormField, OcrLanguages, PageAnnotation, PageLink, SignatureInfo } from "@/ipc/generated/contract";
import { expectAccessible } from "@/test/axe";

/** The window with the first page laid out, as it is once the reader has a size. */
async function showFirstPage(props: Partial<Parameters<typeof ReaderShell>[0]> = {}) {
  const editing = {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  } satisfies EditingApi;
  render(
    <ReaderShell
      state={{ kind: "open", document: { ...demoDocument, doc: 5, hasForm: true } }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      editingApi={editing}
      {...props}
    />,
  );
  const canvas = screen.getByRole("main");
  Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 1000 });
  Object.defineProperty(canvas, "clientHeight", { configurable: true, value: 800 });
  act(() => canvas.dispatchEvent(new Event("scroll")));
  await act(async () => {});
  // The pop-ups of menus let pointer events through only once they have settled: not in jsdom.
  return userEvent.setup({ pointerEventsCheck: PointerEventsCheckLevel.Never });
}

const field = (id: number, label: string, extra: Partial<FormField> = {}): FormField => ({
  id,
  group: id,
  kind: "text",
  rect: { x0: 72, y0: 100 + id * 30, x1: 300, y1: 124 + id * 30 },
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

describe("the page", () => {
  it("a form with a field of every kind", async () => {
    const fields: FormField[] = [
      field(1, "Your name", { value: "Jane" }),
      field(2, "Notes", { multiline: true }),
      field(3, "Secret", { password: true }),
      field(4, "Required", { required: true }),
      field(5, "Locked", { value: "fixed", readOnly: true }),
      field(6, "I agree", { kind: "checkbox", value: "Off", onValue: "Yes" }),
      field(7, "Size", { kind: "radio", group: 30, value: "Off", onValue: "Small" }),
      field(8, "Size", { kind: "radio", group: 30, value: "Off", onValue: "Medium" }),
      field(9, "Country", { kind: "combo", value: "TW", options: [{ value: "TW", label: "Taiwan" }] }),
      field(10, "Fruit", { kind: "list", value: "apple", options: [{ value: "apple", label: "apple" }] }),
    ];
    const formsApi = {
      getPageFields: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? fields : [])),
    } satisfies FormsApi;
    await showFirstPage({ formsApi });
    await screen.findByRole("textbox", { name: "Your name" });
    await expectAccessible(document.body);
  });

  it("the links of a page: inside the document, on the web and blocked", async () => {
    const links: PageLink[] = [
      { id: { pageIndex: 0, index: 0 }, rect: { x0: 72, y0: 100, x1: 300, y1: 122 }, target: { kind: "page", pageIndex: 2, x: null, y: null } },
      { id: { pageIndex: 0, index: 1 }, rect: { x0: 72, y0: 130, x1: 300, y1: 152 }, target: { kind: "uri", uri: "https://example.invalid/docs" } },
      { id: { pageIndex: 0, index: 2 }, rect: { x0: 72, y0: 160, x1: 300, y1: 182 }, target: { kind: "blocked", action: "launch", target: "calc.exe" } },
    ];
    const linksApi = {
      getPageLinks: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? links : [])),
      describeLink: vi.fn(),
      openLink: vi.fn(),
      describeOutlineLink: vi.fn(),
      openOutlineLink: vi.fn(),
    } satisfies LinksApi;
    await showFirstPage({ linksApi });
    await waitFor(() => expect(document.querySelectorAll("[data-link]")).toHaveLength(3));
    await expectAccessible(document.body);
  });
});

describe("marks and tools", () => {
  const marks: PageAnnotation[] = [
    { id: 4, kind: "highlight", rect: { x0: 72, y0: 300, x1: 200, y1: 320 }, color: "yellow", text: null },
    { id: 5, kind: "note", rect: { x0: 300, y0: 300, x1: 320, y1: 320 }, color: null, text: "Existing note" },
    { id: 6, kind: "ink", rect: { x0: 72, y0: 400, x1: 200, y1: 460 }, color: null, text: null },
    { id: 7, kind: "stamp", rect: { x0: 250, y0: 400, x1: 400, y1: 460 }, color: null, text: null },
    { id: 8, kind: "other", rect: { x0: 72, y0: 500, x1: 200, y1: 520 }, color: null, text: null },
  ];
  const annotationsApi = {
    getPageAnnotations: vi.fn((_doc: number, page: number) => Promise.resolve(page === 0 ? marks : [])),
  } satisfies AnnotationsApi;

  it("every kind of mark, as it is on the page and as it is when chosen", async () => {
    await showFirstPage({ annotationsApi });
    await screen.findByRole("button", { name: annotationLabel(marks[0]!) });
    await expectAccessible(document.body);
    for (const mark of marks) {
      act(() => screen.getByRole("button", { name: annotationLabel(mark) }).focus());
      await screen.findByRole("toolbar", { name: annotationLabel(mark) });
      await expectAccessible(document.body);
    }
  });

  it("the pen, with the menu of its colour and thickness", async () => {
    const user = await showFirstPage({ annotationsApi });
    await user.click(screen.getByRole("button", { name: strings.annotations.pen }));
    await expectAccessible(document.body);
    await user.click(screen.getByRole("button", { name: strings.annotations.penStyle }));
    await screen.findByRole("menu");
    await expectAccessible(document.body, {
      skip: { region: "a menu is a pop-up, outside the page's landmarks by design" },
    });
  });

  it("the menu of stamps", async () => {
    const user = await showFirstPage({ annotationsApi });
    await user.click(screen.getByRole("button", { name: strings.annotations.stamp }));
    await screen.findByRole("menu");
    await expectAccessible(document.body, {
      skip: { region: "a menu is a pop-up, outside the page's landmarks by design" },
    });
  });
});

describe("the settings, with everything in them", () => {
  const languages: OcrLanguages = {
    languages: [
      { code: "chi_tra", bundled: true, bytes: 2_366_642n },
      { code: "eng", bundled: true, bytes: 4_113_088n },
      { code: "deu", bundled: false, bytes: 3_145_728n },
    ],
    automatic: "chi_tra",
  };

  it("the dialog, after a check for updates found a newer version", async () => {
    const settingsApi = {
      get: vi.fn<SettingsApi["get"]>(() => Promise.resolve(DEFAULT_SETTINGS)),
      set: vi.fn<SettingsApi["set"]>(() => Promise.resolve()),
    } satisfies SettingsApi;
    const recentApi = {
      list: vi.fn<RecentApi["list"]>(() => Promise.resolve([])),
      open: vi.fn<RecentApi["open"]>(() => Promise.resolve()),
      remove: vi.fn<RecentApi["remove"]>(() => Promise.resolve([])),
      clear: vi.fn<RecentApi["clear"]>(() => Promise.resolve()),
      clearExclusions: vi.fn<RecentApi["clearExclusions"]>(() => Promise.resolve()),
      isRecorded: vi.fn<RecentApi["isRecorded"]>(() => Promise.resolve(true)),
      setRecorded: vi.fn<RecentApi["setRecorded"]>(() => Promise.resolve()),
    } satisfies RecentApi;
    const updatesApi = {
      check: vi.fn<UpdatesApi["check"]>(() => Promise.resolve({ kind: "available", current: "0.1.0", latest: "0.2.0" })),
      describeReleasesPage: vi.fn<UpdatesApi["describeReleasesPage"]>(),
      openReleasesPage: vi.fn<UpdatesApi["openReleasesPage"]>(),
    } satisfies UpdatesApi;
    const ocrApi = {
      languages: vi.fn<OcrApi["languages"]>(() => Promise.resolve(languages)),
      importLanguage: vi.fn<OcrApi["importLanguage"]>(() => Promise.resolve({ kind: "cancelled" })),
      removeLanguage: vi.fn<OcrApi["removeLanguage"]>(() => Promise.resolve(languages)),
      start: vi.fn<OcrApi["start"]>(() => Promise.resolve()),
      stop: vi.fn<OcrApi["stop"]>(() => Promise.resolve()),
      focus: vi.fn<OcrApi["focus"]>(() => Promise.resolve()),
    } satisfies OcrApi;
    const user = userEvent.setup();
    render(
      <SettingsProvider api={settingsApi}>
        <ReaderShell
          state={{ kind: "open", document: { ...demoDocument, doc: 5 } }}
          onOpen={vi.fn()}
          loadingDelayMs={0}
          recentApi={recentApi}
          updatesApi={updatesApi}
          ocrApi={ocrApi}
        />
      </SettingsProvider>,
    );
    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
    await user.click(await screen.findByRole("menuitem", { name: strings.menu.settings }));
    await screen.findByRole("combobox", { name: strings.ocr.settings.language });
    await user.click(screen.getByRole("button", { name: strings.settings.checkUpdates }));
    await screen.findByText(strings.settings.available("0.2.0", "0.1.0"));
    await expectAccessible(document.body);
  });
});

describe("the menus of a right click", () => {
  const popup = { region: "a menu is a pop-up, outside the page's landmarks by design" };

  it("on the page", async () => {
    await showFirstPage();
    fireEvent.contextMenu(screen.getByRole("img", { name: "第 1 頁" }).parentElement!);
    await screen.findByRole("menu");
    await expectAccessible(document.body, { skip: popup });
  });

  it("on a thumbnail", async () => {
    const user = await showFirstPage();
    await user.click(screen.getByRole("tab", { name: strings.sidebar.thumbnailsTab }));
    fireEvent.contextMenu(screen.getByRole("option", { name: strings.canvas.page(2) }));
    await screen.findAllByRole("menuitem");
    await expectAccessible(document.body, { skip: popup });
  });
});

describe("files dragged over the window", () => {
  it("the window says where they will go", async () => {
    render(<ReaderShell state={{ kind: "empty" }} onOpen={vi.fn()} dropActive loadingDelayMs={0} />);
    await expectAccessible(document.body);
  });
});

describe("the digital signatures of a document", () => {
  const signature = (overrides: Partial<SignatureInfo> = {}): SignatureInfo => ({
    status: "valid",
    signerTrusted: false,
    reason: null,
    fieldName: "Signature1",
    signer: "Jane Public",
    claimedTime: "2026-01-01 00:00:00 UTC",
    certification: null,
    ...overrides,
  });
  const view = (...signatures: SignatureInfo[]): SignatureView => ({
    status: "ready",
    report: { signatures, truncated: false },
  });

  it("the banner and the panel, for every state a signature can be in", async () => {
    const states: SignatureInfo[] = [
      signature({ signerTrusted: true }),
      signature(),
      signature({ status: "changedAfterSigning" }),
      signature({ status: "invalid", signer: null }),
      signature({ status: "unverifiable", signer: null, reason: "unsupportedFormat" }),
      signature({ status: "unverifiable", signer: null, reason: "tooLarge" }),
      signature({ certification: "noChanges" }),
    ];
    for (const state of states) {
      const { unmount } = render(
        <ReaderShell
          state={{ kind: "open", document: { ...demoDocument, doc: 5, session: 1 } }}
          onOpen={vi.fn()}
          loadingDelayMs={0}
          signatures={view(state)}
        />,
      );
      await screen.findByRole("region", { name: strings.signatures.label });
      await expectAccessible(document.body);
      const user = userEvent.setup();
      await user.click(screen.getByRole("button", { name: strings.signatures.details }));
      await screen.findByRole("complementary", { name: strings.signatures.detailsTitle });
      await expectAccessible(document.body);
      unmount();
    }
  });
});
