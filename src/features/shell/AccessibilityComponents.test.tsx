import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, vi } from "vitest";

import { NoteDialog } from "@/features/annotations/NoteDialog";
import { BatchDialog } from "@/features/batch/BatchDialog";
import { TextBoxDialog } from "@/features/annotations/TextBoxDialog";
import { ExportDialog } from "@/features/export/ExportDialog";
import { FlattenDialog } from "@/features/forms/FlattenDialog";
import { BlockedLinkDialog, LinkConfirmDialog } from "@/features/links/LinkDialogs";
import { OutlineTree } from "@/features/outline/OutlineTree";
import { PrintDialog } from "@/features/print/PrintDialog";
import { RecoveryBanner } from "@/features/recovery/RecoveryBanner";
import { CreatePdfDialog } from "@/features/create/CreatePdfDialog";
import { CompressDialog } from "@/features/saving/CompressDialog";
import { EncryptDialog } from "@/features/saving/EncryptDialog";
import { PrivacyExportDialog } from "@/features/saving/PrivacyExportDialog";
import { SanitizeDialog } from "@/features/saving/SanitizeDialog";
import { SaveFailedDialog } from "@/features/saving/SaveFailedDialog";
import { SensitiveDialog } from "@/features/sensitive/SensitiveDialog";
import { SensitiveSection } from "@/features/sensitive/SensitiveSection";
import { UnsavedDialog } from "@/features/saving/UnsavedDialog";
import { demoDocument } from "@/features/shell/demo";
import { SetDefaultFailedDialog } from "@/features/shell/dialogs";
import { StatusBar } from "@/features/shell/StatusBar";
import { tabElementId, tabPanelId, type Tab } from "@/features/tabs/model";
import { TabBar } from "@/features/tabs/TabBar";
import { MovePagesDialog } from "@/features/thumbnails/MovePagesDialog";
import { SourcePasswordDialog } from "@/features/thumbnails/SourcePasswordDialog";
import { UndoPasswordDialog } from "@/features/thumbnails/UndoPasswordDialog";
import { RedactionConfirmDialog } from "@/features/redaction/RedactionConfirmDialog";
import { RedactionPanel } from "@/features/redaction/RedactionPanel";
import { strings } from "@/i18n/zh-TW";
import type { BatchEvent, BlockedAction, FontInfo, LinkPreview, OutlineItem } from "@/ipc/generated/contract";
import { expectAccessible } from "@/test/axe";

/** The page, with whatever a dialog put outside the render container. */
const everything = () => document.body;

const FONTS: FontInfo[] = [
  { id: 1, name: "Arial", family: "Arial", bold: false, italic: false, source: "installed" },
  { id: 2, name: "Arial Bold", family: "Arial", bold: true, italic: false, source: "installed" },
  { id: 3, name: "Microsoft JhengHei", family: "Microsoft JhengHei", bold: false, italic: false, source: "installed" },
];

describe("dialogs", () => {
  it("a note", async () => {
    render(<NoteDialog initial={null} onSave={vi.fn()} onCancel={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("a new text box", async () => {
    render(<TextBoxDialog mode="add" fonts={FONTS} onSave={() => Promise.resolve(null)} onCancel={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("a text box whose font is gone, with a problem shown", async () => {
    render(
      <TextBoxDialog
        mode="edit"
        fonts={FONTS}
        start={{
          text: "Hello 你好",
          font: "Gone Font",
          style: { size: 18, color: { r: 200, g: 30, b: 30 }, align: "center" },
          overflow: true,
        }}
        onSave={() => Promise.resolve("problem")}
        onCancel={vi.fn()}
      />,
    );
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByRole("button", { name: strings.annotations.textBoxDialog.save }));
    await screen.findByRole("alert");
    await expectAccessible(everything());
  });

  it("flattening the form", async () => {
    render(<FlattenDialog onConfirm={vi.fn()} onCancel={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("redaction confirmation dialog", async () => {
    render(
      <RedactionConfirmDialog
        open={true}
        summary={{
          marks: 3,
          pages: 2,
          annotations: 1,
          fields: 2,
        }}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("a save that failed", async () => {
    render(<SaveFailedDialog failure={{ code: "readOnly" }} onSaveAs={vi.fn()} onClose={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("documents with unsaved changes", async () => {
    render(
      <UnsavedDialog
        names={["報告.pdf", "合約.pdf"]}
        onSave={() => Promise.resolve()}
        onDiscard={vi.fn()}
        onCancel={vi.fn()}
      />,
    );
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("the privacy export", async () => {
    render(
      <PrivacyExportDialog open onOpenChange={vi.fn()} onExport={() => Promise.resolve(false)} onFinished={vi.fn()} />,
    );
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("the sanitized copy dialog", async () => {
    render(
      <SanitizeDialog
        open
        onOpenChange={vi.fn()}
        doc={1}
        sensitive
        onSanitize={() => Promise.resolve(null)}
      />,
    );
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("the batch, with its choices and with how each file went", async () => {
    let onEvent: (event: BatchEvent) => void = () => {};
    const api = {
      run: vi.fn((_operation: unknown, handler: (event: BatchEvent) => void) => {
        onEvent = handler;
        return { request: 3, done: new Promise<never>(() => {}) };
      }),
      cancel: vi.fn(() => Promise.resolve()),
    };
    const user = userEvent.setup();
    render(<BatchDialog open onOpenChange={vi.fn()} api={api} />);
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("radio", { name: strings.batch.encrypt }));
    await expectAccessible(everything());
    await user.click(within(dialog).getByRole("radio", { name: strings.batch.watermark }));
    await expectAccessible(everything());
    await user.click(within(dialog).getByRole("radio", { name: strings.batch.removeMetadata }));
    await user.click(within(dialog).getByRole("button", { name: strings.batch.start }));
    act(() => {
      onEvent({ kind: "started", files: ["a.pdf", "b.pdf"] });
      onEvent({ kind: "file", index: 0, outcome: { kind: "failed", error: "encrypted" } });
    });
    await expectAccessible(everything());
  });

  it("the encrypted copy", async () => {
    render(<EncryptDialog open onOpenChange={vi.fn()} doc={5} onEncrypt={() => Promise.resolve(false)} onFinished={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("the compressed copy", async () => {
    render(<CompressDialog open onOpenChange={vi.fn()} doc={5} onCompress={() => Promise.resolve(null)} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("what came of the compressed copy", async () => {
    const done = { before: 4_194_304, after: 1_048_576, images: 12, changed: 9, partial: false };
    render(<CompressDialog open onOpenChange={vi.fn()} doc={5} onCompress={() => Promise.resolve(done)} />);
    await userEvent.setup().click(await screen.findByRole("button", { name: strings.compress.start }));
    await screen.findByRole("button", { name: strings.compress.close });
    await expectAccessible(everything());
  });

  it("marking a document as sensitive: ready, with the permissions password and its error, and refused", async () => {
    const ready = { check: vi.fn(() => Promise.resolve("ready" as const)), set: vi.fn() };
    const { unmount } = render(
      <SensitiveDialog open onOpenChange={vi.fn()} doc={5} sensitive={false} api={ready} onFinished={vi.fn()} />,
    );
    await screen.findByText(strings.sensitive.ready);
    await expectAccessible(everything());
    unmount();

    const locked = {
      check: vi.fn(() => Promise.resolve("needsPassword" as const)),
      set: vi.fn(() => Promise.reject({ code: "notAllowed", message: "" })),
    };
    const second = render(
      <SensitiveDialog open onOpenChange={vi.fn()} doc={5} sensitive={false} api={locked} onFinished={vi.fn()} />,
    );
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText(strings.sensitive.passwordLabel), "user");
    await user.click(screen.getByRole("button", { name: strings.sensitive.mark }));
    await screen.findByText(strings.sensitive.notOwnerPassword);
    await expectAccessible(everything());
    second.unmount();

    const signed = { check: vi.fn(() => Promise.resolve("signed" as const)), set: vi.fn() };
    render(<SensitiveDialog open onOpenChange={vi.fn()} doc={5} sensitive api={signed} onFinished={vi.fn()} />);
    await screen.findByText(strings.sensitive.signed);
    await expectAccessible(everything());
  });

  it("the PDF made of files, at rest and with a reason it failed", async () => {
    const api = { createPdf: vi.fn(() => Promise.reject({ code: "corrupted", message: "" })) };
    render(<CreatePdfDialog open onOpenChange={vi.fn()} api={api} onFinished={vi.fn()} />);
    const dialog = await screen.findByRole("dialog");
    await expectAccessible(everything());
    await userEvent.click(within(dialog).getByRole("button", { name: strings.create.start }));
    await within(dialog).findByRole("alert");
    await expectAccessible(everything());
  });

  it("the export", async () => {
    const api = { exportPages: vi.fn(), cancel: vi.fn() };
    render(
      <ExportDialog open onOpenChange={vi.fn()} doc={5} pageCount={10} currentPage={3} api={api} onFinished={vi.fn()} />,
    );
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("the print", async () => {
    render(
      <PrintDialog
        open
        onOpenChange={vi.fn()}
        pageCount={10}
        currentPage={3}
        prepare={() => new Promise(() => {})}
        onReady={vi.fn()}
      />,
    );
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("moving pages", async () => {
    render(<MovePagesDialog open onOpenChange={vi.fn()} pageCount={10} moving={2} onMove={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("the password of a file whose pages are taken in, and when it was wrong", async () => {
    const { rerender } = render(<SourcePasswordDialog open wrong={false} onSubmit={vi.fn()} onCancel={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
    rerender(<SourcePasswordDialog open wrong onSubmit={vi.fn()} onCancel={vi.fn()} />);
    await expectAccessible(everything());
  });

  it("the password that an undo needs, and when it was wrong", async () => {
    const { rerender } = render(<UndoPasswordDialog open wrong={false} onUndo={vi.fn()} onCancel={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
    rerender(<UndoPasswordDialog open wrong onUndo={vi.fn()} onCancel={vi.fn()} />);
    await expectAccessible(everything());
  });

  it("the default reader that could not be set", async () => {
    render(<SetDefaultFailedDialog open onOpenChange={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });
});

describe("links", () => {
  const web = (uri: string, more: Partial<LinkPreview> = {}): LinkPreview => ({
    uri,
    opens: uri,
    host: "example.invalid",
    asciiHost: null,
    ...more,
  });

  it("the confirmation, plain and with warnings", async () => {
    const onOpen = () => Promise.resolve();
    const { rerender } = render(
      <LinkConfirmDialog preview={web("https://example.invalid/docs")} onOpen={onOpen} onClose={vi.fn()} />,
    );
    await screen.findByRole("dialog");
    await expectAccessible(everything());
    const lookalike = web("https://аpple.example.invalid/‮fdp.exe", {
      host: "аpple.example.invalid",
      asciiHost: "xn--pple-43d.example.invalid",
    });
    rerender(<LinkConfirmDialog preview={lookalike} onOpen={onOpen} onClose={vi.fn()} />);
    await expectAccessible(everything());
  });

  it("a link that was blocked, for every reason", async () => {
    const actions: BlockedAction[] = [
      "launch",
      "remoteGoTo",
      "embeddedGoTo",
      "javaScript",
      "submitForm",
      "importData",
      "localFile",
      "networkShare",
      "other",
    ];
    const { rerender } = render(<BlockedLinkDialog action="launch" content="calc.exe" onClose={vi.fn()} />);
    await screen.findByRole("dialog");
    for (const action of actions) {
      rerender(<BlockedLinkDialog action={action} content={action === "other" ? null : "x"} onClose={vi.fn()} />);
      await expectAccessible(everything());
    }
  });
});

describe("what the window shows", () => {
  it("the recovery offer, in each of its four cases", async () => {
    for (const recovery of ["available", "partial", "lost", "stale"] as const) {
      const { unmount } = render(
        <RecoveryBanner recovery={recovery} busy={false} onRestore={vi.fn()} onDiscard={vi.fn()} onLater={vi.fn()} />,
      );
      await expectAccessible(everything());
      unmount();
    }
  });

  it("the tabs, whatever each one holds", async () => {
    const tabs: Tab[] = [
      { tab: 1, displayName: "報告.pdf", content: { kind: "open", document: demoDocument, hasOutline: true } },
      { tab: 2, displayName: "合約.pdf", content: { kind: "loading" } },
      { tab: 3, displayName: "機密.pdf", content: { kind: "password", wrong: false, canRemember: true, rememberedFailed: false } },
      { tab: 4, displayName: "壞掉.pdf", content: { kind: "error", code: "corrupted" } },
    ];
    // The window's own panels hold the reader; here they only have to be what the tabs point to.
    render(
      <>
        <TabBar tabs={tabs} active={1} onActivate={vi.fn()} onClose={vi.fn()} onOpen={vi.fn()} />
        {tabs.map(({ tab }) => (
          <div key={tab} role="tabpanel" id={tabPanelId(tab)} aria-labelledby={tabElementId(tab)} hidden />
        ))}
      </>,
    );
    await expectAccessible(everything());
  });

  it("the status bar, with a hint, a restriction and the recognition of text", async () => {
    render(
      <StatusBar
        document={{ displayName: "報告.pdf", currentPage: 2, pageCount: 10, zoom: "fitWidth" }}
        hoverTarget="https://example.invalid/"
        hint="這份文件不允許複製文字"
        restriction="已限制：不可複製、不可列印"
        ocr={{ text: strings.ocr.running(3, 8), onStop: vi.fn() }}
        ocrNote={strings.ocr.pageNote}
        sensitive
      />,
    );
    await expectAccessible(everything());
  });

  it("the list of files marked as sensitive", async () => {
    const api = {
      list: vi.fn(() =>
        Promise.resolve([
          { id: 1, displayName: "合約.pdf", open: true },
          { id: 2, displayName: "薪資.pdf", open: false },
        ]),
      ),
      remove: vi.fn(),
    };
    render(<SensitiveSection api={api} />);
    await screen.findByText("合約.pdf");
    await expectAccessible(everything());
  });

  it("the outline", async () => {
    const page = (pageIndex: number) => ({ kind: "page" as const, pageIndex, x: null, y: null });
    const items: OutlineItem[] = [
      { id: 1, title: "第一章", depth: 0, target: page(0) },
      { id: 2, title: "第一節", depth: 1, target: page(1) },
      { id: 3, title: "第二章", depth: 0, target: page(3) },
      { id: 4, title: "網站", depth: 0, target: { kind: "uri", uri: "https://example.invalid/" } },
      { id: 5, title: "已封鎖", depth: 0, target: { kind: "blocked", action: "launch", target: "calc.exe" } },
    ];
    render(<OutlineTree items={items} currentPage={1} onJumpToPage={vi.fn()} onOpenLink={vi.fn()} />);
    await expectAccessible(everything(), {
      skip: { region: "the tree is shown without the sidebar that has it in the window" },
    });
  });

  it("the redaction panel", async () => {
    const marks = [
      {
        page: 0,
        annotation: {
          id: 1,
          kind: "redaction" as const,
          rect: { x0: 10, y0: 10, x1: 100, y1: 100 },
          color: null,
          text: null,
          picture: null,
        },
      },
    ];
    render(
      <RedactionPanel
        marks={marks}
        onDeleteMark={vi.fn()}
        onDeleteAll={vi.fn()}
        onApply={vi.fn()}
        onJumpToPage={vi.fn()}
      />,
    );
    await expectAccessible(everything(), {
      skip: { region: "the panel is shown without the sidebar that has it in the window" },
    });
  });
});
