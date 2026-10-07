import { render, screen } from "@testing-library/react";
import { describe, it, vi } from "vitest";

import { NoteDialog } from "@/features/annotations/NoteDialog";
import { ExportDialog } from "@/features/export/ExportDialog";
import { FlattenDialog } from "@/features/forms/FlattenDialog";
import { BlockedLinkDialog, LinkConfirmDialog } from "@/features/links/LinkDialogs";
import { OutlineTree } from "@/features/outline/OutlineTree";
import { PrintDialog } from "@/features/print/PrintDialog";
import { RecoveryBanner } from "@/features/recovery/RecoveryBanner";
import { PrivacyExportDialog } from "@/features/saving/PrivacyExportDialog";
import { SaveFailedDialog } from "@/features/saving/SaveFailedDialog";
import { UnsavedDialog } from "@/features/saving/UnsavedDialog";
import { demoDocument } from "@/features/shell/demo";
import { SetDefaultFailedDialog } from "@/features/shell/dialogs";
import { StatusBar } from "@/features/shell/StatusBar";
import { tabElementId, tabPanelId, type Tab } from "@/features/tabs/model";
import { TabBar } from "@/features/tabs/TabBar";
import { MovePagesDialog } from "@/features/thumbnails/MovePagesDialog";
import { SourcePasswordDialog } from "@/features/thumbnails/SourcePasswordDialog";
import { UndoPasswordDialog } from "@/features/thumbnails/UndoPasswordDialog";
import { strings } from "@/i18n/zh-TW";
import type { BlockedAction, LinkPreview, OutlineItem } from "@/ipc/generated/contract";
import { expectAccessible } from "@/test/axe";

/** The page, with whatever a dialog put outside the render container. */
const everything = () => document.body;

describe("dialogs", () => {
  it("a note", async () => {
    render(<NoteDialog initial={null} onSave={vi.fn()} onCancel={vi.fn()} />);
    await screen.findByRole("dialog");
    await expectAccessible(everything());
  });

  it("flattening the form", async () => {
    render(<FlattenDialog onConfirm={vi.fn()} onCancel={vi.fn()} />);
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
      { tab: 3, displayName: "機密.pdf", content: { kind: "password", wrong: false } },
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
    await expectAccessible(everything(), {
      skip: { "aria-required-children": "the close buttons are in the tablist: #188" },
    });
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
      />,
    );
    await expectAccessible(everything());
  });

  it("the outline", async () => {
    const page = (pageIndex: number) => ({ kind: "page" as const, pageIndex, x: null, y: null });
    const items: OutlineItem[] = [
      { title: "第一章", depth: 0, target: page(0) },
      { title: "第一節", depth: 1, target: page(1) },
      { title: "第二章", depth: 0, target: page(3) },
      { title: "網站", depth: 0, target: { kind: "uri", uri: "https://example.invalid/" } },
      { title: "已封鎖", depth: 0, target: { kind: "blocked", action: "launch", target: "calc.exe" } },
    ];
    render(<OutlineTree items={items} currentPage={1} onJumpToPage={vi.fn()} onOpenLink={vi.fn()} />);
    await expectAccessible(everything(), {
      skip: { region: "the tree is shown without the sidebar that has it in the window" },
    });
  });
});
