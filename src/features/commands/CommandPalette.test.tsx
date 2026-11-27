import { useState } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/tooltip";
import { CommandPalette } from "@/features/commands/CommandPalette";
import { clearRecentCommands, createCommands } from "@/features/commands/registry";
import type { CommandOptions } from "@/features/commands/types";
import { strings } from "@/i18n";

function makeOptions(opts: Partial<CommandOptions> = {}): CommandOptions {
  return {
    document: { pageCount: 5, currentPage: 1, zoom: 100 },
    sidebarOpen: false,
    searchOpen: false,
    theme: "system",
    onToggleSidebar: vi.fn(),
    onOpen: vi.fn(),
    onGoToPage: vi.fn(),
    onZoomIn: vi.fn(),
    onZoomOut: vi.fn(),
    onZoomChange: vi.fn(),
    onRotate: vi.fn(),
    onSearch: vi.fn(),
    onThemeChange: vi.fn(),
    onShowShortcuts: vi.fn(),
    onShowAbout: vi.fn(),
    onSetDefault: vi.fn(),
    onSave: vi.fn(),
    onSaveAs: vi.fn(),
    onPrint: vi.fn(),
    onUndo: vi.fn(),
    onRedo: vi.fn(),
    onCreatePdf: vi.fn(),
    onExport: vi.fn(),
    onExportAnnotations: vi.fn(),
    onExportXfdf: vi.fn(),
    onCompareText: vi.fn(),
    onCompareVisual: vi.fn(),
    onPrivacyExport: vi.fn(),
    onSanitizeCopy: vi.fn(),
    onPageMarks: vi.fn(),
    onEncryptCopy: vi.fn(),
    onSearchAll: vi.fn(),
    onBatch: vi.fn(),
    onForgetPassword: vi.fn(),
    onCompressImages: vi.fn(),
    onSign: vi.fn(),
    onClose: vi.fn(),
    onCopy: vi.fn(),
    onFindNext: vi.fn(),
    onFindPrevious: vi.fn(),
    onFitPage: vi.fn(),
    onActualSize: vi.fn(),
    onFitWidth: vi.fn(),
    onFirstPage: vi.fn(),
    onLastPage: vi.fn(),
    onPresentation: vi.fn(),
    onPresentationCurrent: vi.fn(),
    onNextRegion: vi.fn(),
    onPreviousRegion: vi.fn(),
    onLayoutModeChange: vi.fn(),
    onToggleFullscreen: vi.fn(),
    ocr: { running: false, onStart: vi.fn() },
    onSearchableCopy: vi.fn(),
    flatten: { onClick: vi.fn() },
    sensitive: { marked: false, onClick: vi.fn() },
    recording: { recorded: true, onChange: vi.fn() },
    onInsertFrom: vi.fn(),
    insertFromBlocked: false,
    ...opts,
  };
}

function setup(opts: Partial<CommandOptions> = {}, defaultOpen = true) {
  const user = userEvent.setup();
  const onOpenChange = vi.fn();

  const options = makeOptions(opts);

  const commands = createCommands(options);

  const utils = render(
    <TooltipProvider>
      <CommandPalette open={defaultOpen} onOpenChange={onOpenChange} commands={commands} />
    </TooltipProvider>,
  );

  return { ...utils, user, commands, onOpenChange, options };
}

describe("CommandPalette", () => {
  beforeEach(() => {
    clearRecentCommands();
  });

  describe("completeness of menu commands", () => {
    it("contains all 39 commands from the More menu", () => {
      const { commands } = setup();
      const menuCommands = commands.filter((c) => c.fromMenu);
      expect(menuCommands).toHaveLength(39);

      // Verify key More menu items are present
      const names = menuCommands.map((c) => c.name);
      expect(names).toContain(strings.menu.save);
      expect(names).toContain(strings.menu.saveAs);
      expect(names).toContain(strings.menu.undo);
      expect(names).toContain(strings.menu.redo);
      expect(names).toContain(strings.menu.export);
      expect(names).toContain(strings.menu.watermark);
      expect(names).toContain(strings.menu.print);
      expect(names).toContain(strings.menu.compareVisual);
      expect(names).toContain(strings.menu.documentProperties);
      expect(names).toContain(strings.menu.fullscreen);
      expect(names).toContain(strings.menu.shortcuts);
      expect(names).toContain(strings.menu.setDefault);
      expect(names).toContain(strings.menu.decryptCopy);
    });

    it("contains all 26 shortcut/viewer commands and 2 palette-only commands", () => {
      const { commands } = setup();
      const shortcutCommands = commands.filter(
        (c) => !c.fromMenu && c.id !== "insertFileAfterCurrent" && c.id !== "replaceCurrentPage",
      );
      expect(shortcutCommands).toHaveLength(26);

      const paletteCommands = commands.filter((c) => c.id === "insertFileAfterCurrent" || c.id === "replaceCurrentPage");
      expect(paletteCommands).toHaveLength(2);

      const names = shortcutCommands.map((c) => c.name);
      expect(names).toContain(strings.shortcuts.descriptions.zoomIn);
      expect(names).toContain(strings.shortcuts.descriptions.rotateCw);
      expect(names).toContain(strings.shortcuts.descriptions.toggleSidebar);
      expect(names).toContain(strings.shortcuts.descriptions.fitWidth);
    });

    it("filters and executes compareVisual", async () => {
      const onCompareVisual = vi.fn();
      const { user, onOpenChange } = setup({ onCompareVisual });
      const input = screen.getByRole("combobox");

      await user.type(input, "畫面");
      const option = screen.getByRole("option", { name: new RegExp(strings.menu.compareVisual) });
      await user.click(option);

      expect(onCompareVisual).toHaveBeenCalled();
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });
  });

  describe("filtering", () => {
    it("filters by Chinese substring", async () => {
      const { user } = setup();
      const input = screen.getByRole("combobox");

      await user.type(input, "旋轉");
      const options = screen.getAllByRole("option");
      expect(options.length).toBeGreaterThan(0);
      for (const opt of options) {
        expect(opt.textContent).toMatch(/旋轉/);
      }
    });

    it("filters by alias", async () => {
      const { user } = setup();
      const input = screen.getByRole("combobox");

      // "合併" is an alias for "在目前這一頁後面插入其他檔案的頁面…"
      await user.type(input, "合併");
      const options = screen.getAllByRole("option");
      expect(options.length).toBeGreaterThan(0);
      expect(options[0]).toHaveTextContent(strings.commands.insertFileAfterCurrent);

      // "密碼" is an alias for "加密並另存新檔…"
      await user.clear(input);
      await user.type(input, "密碼");
      const passwordOptions = screen.getAllByRole("option");
      expect(passwordOptions.some((o) => o.textContent?.includes(strings.menu.encryptCopy))).toBe(true);
    });

    it("filters by English case-insensitively", async () => {
      const { user } = setup();
      const input = screen.getByRole("combobox");

      await user.type(input, "pdf");
      const optionsLower = screen.getAllByRole("option");
      expect(optionsLower.length).toBeGreaterThan(0);

      await user.clear(input);
      await user.type(input, "PDF");
      const optionsUpper = screen.getAllByRole("option");
      expect(optionsUpper.length).toBe(optionsLower.length);

      await user.clear(input);
      await user.type(input, "xfdf");
      const xfdfOptions = screen.getAllByRole("option");
      expect(xfdfOptions.some((o) => o.textContent?.includes(strings.menu.exportXfdf))).toBe(true);
    });

    it("displays no results when nothing matches", async () => {
      const { user } = setup();
      const input = screen.getByRole("combobox");

      await user.type(input, "zzzznotfoundword");
      expect(screen.queryByRole("option")).toBeNull();
      expect(screen.getByRole("status")).toHaveTextContent(strings.commands.noResults);
    });
  });

  describe("recent commands sorting", () => {
    it("sorts recently executed commands to the top", async () => {
      const { user, commands, onOpenChange } = setup();
      const input = screen.getByRole("combobox");

      // Execute "旋轉" command
      await user.type(input, "順時針旋轉");
      await user.keyboard("{Enter}");

      expect(onOpenChange).toHaveBeenCalledWith(false);

      // Re-render palette (as if reopened)
      render(
        <TooltipProvider>
          <CommandPalette open={true} onOpenChange={vi.fn()} commands={commands} />
        </TooltipProvider>,
      );

      // The first item should now be "順時針旋轉"
      const options = screen.getAllByRole("option");
      expect(options[0]).toHaveTextContent("順時針旋轉");
    });
  });

  describe("disabled reasons and states", () => {
    it("disables document-required commands when document is null and shows reason", () => {
      setup({ document: null });
      const options = screen.getAllByRole("option");

      // "儲存" should be disabled with reason "需要先開啟文件"
      const saveOption = options.find((o) => o.textContent?.includes("儲存"));
      expect(saveOption).toHaveAttribute("aria-disabled", "true");
      expect(saveOption).toHaveTextContent(strings.commands.noDocument);

      // "開啟檔案" should still be enabled
      const openOption = options.find((o) => o.textContent?.includes("開啟檔案"));
      expect(openOption).toHaveAttribute("aria-disabled", "false");
    });

    it("shows permissions blocked reason and prevents execution", async () => {
      const onPrint = vi.fn();
      const { user, onOpenChange } = setup({ printBlocked: true, onPrint });
      const input = screen.getByRole("combobox");

      await user.type(input, "列印…");
      const printOption = screen.getByRole("option", { name: /列印…/ });
      expect(printOption).toHaveAttribute("aria-disabled", "true");
      expect(printOption).toHaveTextContent(strings.permissions.notAllowed);

      // Pressing Enter or clicking should not run onPrint or close dialog
      await user.click(printOption);
      expect(onPrint).not.toHaveBeenCalled();
      expect(onOpenChange).not.toHaveBeenCalled();
    });

    it("disables encryptCopy and compressImages with signed reason and prevents execution", async () => {
      const onEncryptCopy = vi.fn();
      const onCompressImages = vi.fn();
      expect(strings.encryption.signedBlocked).toBeTruthy();
      expect(strings.compress.signedBlocked).toBeTruthy();
      const { user, onOpenChange } = setup({
        encryptCopyBlocked: strings.encryption.signedBlocked,
        onEncryptCopy,
        compressImagesBlocked: strings.compress.signedBlocked,
        onCompressImages,
      });
      const input = screen.getByRole("combobox");

      await user.type(input, strings.menu.encryptCopy);
      const encryptOption = screen.getByRole("option", { name: new RegExp(strings.menu.encryptCopy) });
      expect(encryptOption).toHaveAttribute("aria-disabled", "true");
      expect(encryptOption).toHaveTextContent(strings.encryption.signedBlocked);
      await user.click(encryptOption);
      expect(onEncryptCopy).not.toHaveBeenCalled();
      expect(onOpenChange).not.toHaveBeenCalled();

      await user.clear(input);
      await user.type(input, strings.menu.compressImages);
      const compressOption = screen.getByRole("option", { name: new RegExp(strings.menu.compressImages) });
      expect(compressOption).toHaveAttribute("aria-disabled", "true");
      expect(compressOption).toHaveTextContent(strings.compress.signedBlocked);
      await user.click(compressOption);
      expect(onCompressImages).not.toHaveBeenCalled();
      expect(onOpenChange).not.toHaveBeenCalled();
    });
  });

  describe("insertFileAfterCurrent command", () => {
    it("finds insertFileAfterCurrent as first item when searching 合併", async () => {
      const { user } = setup();
      const input = screen.getByRole("combobox");

      await user.type(input, "合併");
      const options = screen.getAllByRole("option");
      expect(options[0]).toHaveTextContent(strings.commands.insertFileAfterCurrent);
    });

    it("disables insertFileAfterCurrent with noDocument reason when document is null", () => {
      setup({ document: null });
      const options = screen.getAllByRole("option");
      const insertOption = options.find((o) => o.textContent?.includes(strings.commands.insertFileAfterCurrent));
      expect(insertOption).toHaveAttribute("aria-disabled", "true");
      expect(insertOption).toHaveTextContent(strings.commands.noDocument);
    });

    it("disables insertFileAfterCurrent with permissions not allowed when insertFromBlocked is true", async () => {
      const onInsertFrom = vi.fn();
      const { user } = setup({ insertFromBlocked: true, onInsertFrom });
      const input = screen.getByRole("combobox");

      await user.type(input, "合併");
      const options = screen.getAllByRole("option");
      expect(options[0]).toHaveAttribute("aria-disabled", "true");
      expect(options[0]).toHaveTextContent(strings.permissions.notAllowed);

      await user.click(options[0]!);
      expect(onInsertFrom).not.toHaveBeenCalled();
    });

    it("executes insertFrom with correct at (currentPage) when enabled", async () => {
      const onInsertFrom = vi.fn();
      const { user, onOpenChange } = setup({
        document: { pageCount: 10, currentPage: 4, zoom: 100 },
        insertFromBlocked: false,
        onInsertFrom,
      });
      const input = screen.getByRole("combobox");

      await user.type(input, "合併");
      const options = screen.getAllByRole("option");
      await user.click(options[0]!);

      expect(onInsertFrom).toHaveBeenCalledWith(4);
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });
  });

  describe("insertFileAfterCurrent hint on failure (#584)", () => {
    it("calls onHint with sourceNotAllowed when onInsertFrom rejects", async () => {
      const onInsertFrom = vi.fn().mockRejectedValue({ code: "notAllowed", message: "x" });
      const onHint = vi.fn();
      const options = makeOptions({
        document: { pageCount: 5, currentPage: 3, zoom: 100 },
        onInsertFrom,
        onHint,
      });
      const commands = createCommands(options);
      const cmd = commands.find((c) => c.id === "insertFileAfterCurrent");
      expect(cmd).toBeDefined();

      cmd!.run();

      expect(onInsertFrom).toHaveBeenCalledWith(3);
      await vi.waitFor(() => {
        expect(onHint).toHaveBeenCalledTimes(1);
        expect(onHint).toHaveBeenCalledWith(strings.pages.sourceNotAllowed);
      });
    });

    it("does not call onHint when onInsertFrom succeeds", async () => {
      const onInsertFrom = vi.fn().mockResolvedValue(2);
      const onHint = vi.fn();
      const options = makeOptions({
        document: { pageCount: 5, currentPage: 3, zoom: 100 },
        onInsertFrom,
        onHint,
      });
      const commands = createCommands(options);
      const cmd = commands.find((c) => c.id === "insertFileAfterCurrent");
      expect(cmd).toBeDefined();

      cmd!.run();

      expect(onInsertFrom).toHaveBeenCalledWith(3);
      await vi.waitFor(() => {
        expect(onInsertFrom).toHaveReturned();
      });
      await vi.waitFor(async () => {
        await Promise.resolve();
        expect(onHint).not.toHaveBeenCalled();
      });
    });
  });

  describe("replaceCurrentPage command (#590)", () => {
    it("finds replaceCurrentPage when searching 取代", async () => {
      const { user } = setup();
      const input = screen.getByRole("combobox");

      await user.type(input, "取代");
      const options = screen.getAllByRole("option");
      expect(options[0]).toHaveTextContent(strings.commands.replaceCurrentPage);
    });

    it("disables replaceCurrentPage with noDocument reason when document is null", () => {
      setup({ document: null });
      const options = screen.getAllByRole("option");
      const replaceOption = options.find((o) => o.textContent?.includes(strings.commands.replaceCurrentPage));
      expect(replaceOption).toHaveAttribute("aria-disabled", "true");
      expect(replaceOption).toHaveTextContent(strings.commands.noDocument);
    });

    it("disables replaceCurrentPage when insertFromBlocked is true", () => {
      setup({ insertFromBlocked: true });
      const options = screen.getAllByRole("option");
      const replaceOption = options.find((o) => o.textContent?.includes(strings.commands.replaceCurrentPage));
      expect(replaceOption).toHaveAttribute("aria-disabled", "true");
      expect(replaceOption).toHaveTextContent(strings.permissions.notAllowed);
    });

    it("runs onReplacePages with 0-based page index and count 1, and closes palette", async () => {
      const onReplacePages = vi.fn().mockResolvedValue(1);
      const { user, onOpenChange } = setup({
        document: { pageCount: 5, currentPage: 4, zoom: 100 },
        onReplacePages,
      });
      const input = screen.getByRole("combobox");

      await user.type(input, "取代");
      const options = screen.getAllByRole("option");
      await user.click(options[0]!);

      expect(onReplacePages).toHaveBeenCalledWith(3, 1);
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });

    it("calls onHint with sourceNotAllowed when onReplacePages rejects", async () => {
      const onReplacePages = vi.fn().mockRejectedValue({ code: "notAllowed", message: "x" });
      const onHint = vi.fn();
      const options = makeOptions({
        document: { pageCount: 5, currentPage: 3, zoom: 100 },
        onReplacePages,
        onHint,
      });
      const commands = createCommands(options);
      const cmd = commands.find((c) => c.id === "replaceCurrentPage");
      expect(cmd).toBeDefined();

      cmd!.run();

      expect(onReplacePages).toHaveBeenCalledWith(2, 1);
      await vi.waitFor(() => {
        expect(onHint).toHaveBeenCalledTimes(1);
        expect(onHint).toHaveBeenCalledWith(strings.pages.sourceNotAllowed);
      });
    });

    it("does not call onHint when onReplacePages succeeds", async () => {
      const onReplacePages = vi.fn().mockResolvedValue(2);
      const onHint = vi.fn();
      const options = makeOptions({
        document: { pageCount: 5, currentPage: 3, zoom: 100 },
        onReplacePages,
        onHint,
      });
      const commands = createCommands(options);
      const cmd = commands.find((c) => c.id === "replaceCurrentPage");
      expect(cmd).toBeDefined();

      cmd!.run();

      expect(onReplacePages).toHaveBeenCalledWith(2, 1);
      await vi.waitFor(() => {
        expect(onReplacePages).toHaveReturned();
      });
      await vi.waitFor(async () => {
        await Promise.resolve();
        expect(onHint).not.toHaveBeenCalled();
      });
    });
  });

  describe("keyboard navigation and focus", () => {
    it("navigates with Up/Down arrow keys and updates aria-activedescendant", async () => {
      const { user } = setup();
      const input = screen.getByRole("combobox");
      const options = screen.getAllByRole("option");
      expect(options.length).toBeGreaterThan(1);

      expect(input).toHaveAttribute("aria-activedescendant", options[0]?.id);
      expect(options[0]).toHaveAttribute("aria-selected", "true");

      await user.keyboard("{ArrowDown}");
      expect(input).toHaveAttribute("aria-activedescendant", options[1]?.id);
      expect(options[1]).toHaveAttribute("aria-selected", "true");

      await user.keyboard("{ArrowUp}");
      expect(input).toHaveAttribute("aria-activedescendant", options[0]?.id);

      // Home and End keys
      await user.keyboard("{End}");
      expect(input).toHaveAttribute("aria-activedescendant", options[options.length - 1]?.id);

      await user.keyboard("{Home}");
      expect(input).toHaveAttribute("aria-activedescendant", options[0]?.id);
    });

    it("executes active command on Enter", async () => {
      const onRotate = vi.fn();
      const { user, onOpenChange } = setup({ onRotate });
      const input = screen.getByRole("combobox");

      await user.type(input, "順時針旋轉");
      await user.keyboard("{Enter}");

      expect(onRotate).toHaveBeenCalledWith(1);
      expect(onOpenChange).toHaveBeenCalledWith(false);
    });

    it("restores focus to previous element on Escape", async () => {
      const user = userEvent.setup();

      function Harness() {
        const [open, setOpen] = useState(false);
        const commands = createCommands({
          document: null,
          sidebarOpen: false,
          searchOpen: false,
          theme: "system",
          onToggleSidebar: vi.fn(),
          onOpen: vi.fn(),
          onGoToPage: vi.fn(),
          onZoomIn: vi.fn(),
          onZoomOut: vi.fn(),
          onZoomChange: vi.fn(),
          onRotate: vi.fn(),
          onSearch: vi.fn(),
          onThemeChange: vi.fn(),
          onShowShortcuts: vi.fn(),
          onShowAbout: vi.fn(),
          onSetDefault: vi.fn(),
        });

        return (
          <TooltipProvider>
            <button data-testid="trigger" onClick={() => setOpen(true)}>
              Open
            </button>
            <CommandPalette open={open} onOpenChange={setOpen} commands={commands} />
          </TooltipProvider>
        );
      }

      render(<Harness />);
      const trigger = screen.getByTestId("trigger");
      trigger.focus();
      expect(trigger).toHaveFocus();

      await user.click(trigger);

      const combobox = await screen.findByRole("combobox");
      await waitFor(() => expect(combobox).toHaveFocus());

      await user.keyboard("{Escape}");

      await waitFor(() => expect(screen.queryByRole("combobox")).toBeNull());
      expect(trigger).toHaveFocus();
    });

    it("does not restore focus to previous element when command is executed so new dialog keeps focus", async () => {
      const user = userEvent.setup();
      const onAction = vi.fn();

      function Harness() {
        const [open, setOpen] = useState(false);
        const commands = createCommands({
          document: { pageCount: 1, currentPage: 1, zoom: 100 },
          sidebarOpen: false,
          searchOpen: false,
          theme: "system",
          onToggleSidebar: vi.fn(),
          onOpen: vi.fn(),
          onGoToPage: vi.fn(),
          onZoomIn: vi.fn(),
          onZoomOut: vi.fn(),
          onZoomChange: vi.fn(),
          onRotate: vi.fn(),
          onSearch: vi.fn(),
          onThemeChange: vi.fn(),
          onShowShortcuts: onAction,
          onShowAbout: vi.fn(),
          onSetDefault: vi.fn(),
        });

        return (
          <TooltipProvider>
            <button data-testid="trigger" onClick={() => setOpen(true)}>
              Open
            </button>
            <CommandPalette open={open} onOpenChange={setOpen} commands={commands} />
          </TooltipProvider>
        );
      }

      render(<Harness />);
      const trigger = screen.getByTestId("trigger");
      trigger.focus();
      expect(trigger).toHaveFocus();

      await user.click(trigger);

      const combobox = await screen.findByRole("combobox");
      await waitFor(() => expect(combobox).toHaveFocus());

      await user.type(combobox, "鍵盤快捷鍵");
      await user.keyboard("{Enter}");

      expect(onAction).toHaveBeenCalledTimes(1);
      // Wait for any setTimeout(0) to complete
      await new Promise((r) => setTimeout(r, 50));
      expect(trigger).not.toHaveFocus();
    });

    it("does not crash or throw if previous element was removed from document", async () => {
      const user = userEvent.setup();

      function Harness() {
        const [open, setOpen] = useState(false);
        const [showTrigger, setShowTrigger] = useState(true);
        const commands = createCommands({
          document: null,
          sidebarOpen: false,
          searchOpen: false,
          theme: "system",
          onToggleSidebar: vi.fn(),
          onOpen: vi.fn(),
          onGoToPage: vi.fn(),
          onZoomIn: vi.fn(),
          onZoomOut: vi.fn(),
          onZoomChange: vi.fn(),
          onRotate: vi.fn(),
          onSearch: vi.fn(),
          onThemeChange: vi.fn(),
          onShowShortcuts: vi.fn(),
          onShowAbout: vi.fn(),
          onSetDefault: vi.fn(),
        });

        return (
          <TooltipProvider>
            {showTrigger && (
              <button
                data-testid="trigger"
                onClick={() => {
                  setOpen(true);
                  setShowTrigger(false);
                }}
              >
                Open
              </button>
            )}
            <CommandPalette open={open} onOpenChange={setOpen} commands={commands} />
          </TooltipProvider>
        );
      }

      render(<Harness />);
      const trigger = screen.getByTestId("trigger");
      trigger.focus();
      await user.click(trigger);

      const combobox = await screen.findByRole("combobox");
      await waitFor(() => expect(combobox).toHaveFocus());

      await user.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByRole("combobox")).toBeNull());
    });

    it("gives the focus back only to an element from this opening (#622)", async () => {
      const user = userEvent.setup();

      function Harness() {
        const [open, setOpen] = useState(false);
        const commands = createCommands({
          document: null,
          sidebarOpen: false,
          searchOpen: false,
          theme: "system",
          onToggleSidebar: vi.fn(),
          onOpen: vi.fn(),
          onGoToPage: vi.fn(),
          onZoomIn: vi.fn(),
          onZoomOut: vi.fn(),
          onZoomChange: vi.fn(),
          onRotate: vi.fn(),
          onSearch: vi.fn(),
          onThemeChange: vi.fn(),
          onShowShortcuts: vi.fn(),
          onShowAbout: vi.fn(),
          onSetDefault: vi.fn(),
        });

        return (
          <TooltipProvider>
            <button data-testid="first" onClick={() => setOpen(true)}>
              First
            </button>
            <div role="dialog" aria-label="other">
              <button data-testid="inside" onClick={() => setOpen(true)}>
                Inside
              </button>
            </div>
            <CommandPalette open={open} onOpenChange={setOpen} commands={commands} />
          </TooltipProvider>
        );
      }

      render(<Harness />);
      const first = screen.getByTestId("first");
      await user.click(first);
      await screen.findByRole("combobox");
      await user.keyboard("{Escape}");
      await waitFor(() => expect(first).toHaveFocus());

      await user.click(screen.getByTestId("inside"));
      await screen.findByRole("combobox");
      await user.keyboard("{Escape}");
      await waitFor(() => expect(screen.queryByRole("combobox")).toBeNull());
      await new Promise((r) => setTimeout(r, 50));
      expect(first).not.toHaveFocus();
    });
  });
});
