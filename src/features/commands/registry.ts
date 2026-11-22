import { getShortcutKey } from "@/features/shortcuts/registry";
import { insertFailureMessage } from "@/features/thumbnails/insertFailure";
import { strings } from "@/i18n/zh-TW";
import type { Command, CommandId, CommandOptions } from "./types";

let recentCommandIds: string[] = [];

/** Records a recently executed command into memory (most recent first). */
export function recordRecentCommand(id: CommandId | string): void {
  recentCommandIds = [id, ...recentCommandIds.filter((item) => item !== id)].slice(0, 10);
}

/** Returns the in-memory recently executed command IDs. */
export function getRecentCommands(): string[] {
  return [...recentCommandIds];
}

/** Clears recently executed command IDs from memory (for tests or reset). */
export function clearRecentCommands(): void {
  recentCommandIds = [];
}

/**
 * Creates the complete list of commands (36 from the More menu, 25 shortcut-only / viewer actions, 1 palette-only action).
 * Both the "⋯" menu and the Command Palette are generated from this list.
 */
export function createCommands(opts: CommandOptions): Command[] {
  const menu = strings.menu;
  const aliases = strings.commands.aliases;
  const hasDoc = opts.document !== null;

  const commands: Command[] = [
    // --- 1. Page Layout (Menu Section: layout) ---
    {
      id: "layoutSingle",
      name: menu.layoutSingle,
      aliases: [...aliases.layoutSingle],
      fromMenu: true,
      menuSection: "layout",
      menuKind: "radio",
      checked: (opts.layoutMode ?? "single") === "single",
      visibleInMenu: Boolean(opts.onLayoutModeChange),
      enabled: hasDoc && Boolean(opts.onLayoutModeChange),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onLayoutModeChange?.("single"),
    },
    {
      id: "layoutTwoPage",
      name: menu.layoutTwoPage,
      aliases: [...aliases.layoutTwoPage],
      fromMenu: true,
      menuSection: "layout",
      menuKind: "radio",
      checked: opts.layoutMode === "twoPage",
      visibleInMenu: Boolean(opts.onLayoutModeChange),
      enabled: hasDoc && Boolean(opts.onLayoutModeChange),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onLayoutModeChange?.("twoPage"),
    },
    {
      id: "layoutTwoPageCover",
      name: menu.layoutTwoPageCover,
      aliases: [...aliases.layoutTwoPageCover],
      fromMenu: true,
      menuSection: "layout",
      menuKind: "radio",
      checked: opts.layoutMode === "twoPageCover",
      visibleInMenu: Boolean(opts.onLayoutModeChange),
      enabled: hasDoc && Boolean(opts.onLayoutModeChange),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onLayoutModeChange?.("twoPageCover"),
    },

    // --- 2. Appearance & Fullscreen (Menu Section: appearance) ---
    {
      id: "themeSystem",
      name: menu.themeSystem,
      aliases: [...aliases.themeSystem],
      fromMenu: true,
      menuSection: "appearance",
      menuKind: "radio",
      checked: opts.theme === "system",
      visibleInMenu: true,
      enabled: Boolean(opts.onThemeChange),
      run: () => opts.onThemeChange("system"),
    },
    {
      id: "themeLight",
      name: menu.themeLight,
      aliases: [...aliases.themeLight],
      fromMenu: true,
      menuSection: "appearance",
      menuKind: "radio",
      checked: opts.theme === "light",
      visibleInMenu: true,
      enabled: Boolean(opts.onThemeChange),
      run: () => opts.onThemeChange("light"),
    },
    {
      id: "themeDark",
      name: menu.themeDark,
      aliases: [...aliases.themeDark],
      fromMenu: true,
      menuSection: "appearance",
      menuKind: "radio",
      checked: opts.theme === "dark",
      visibleInMenu: true,
      enabled: Boolean(opts.onThemeChange),
      run: () => opts.onThemeChange("dark"),
    },
    {
      id: "fullscreen",
      name: menu.fullscreen,
      aliases: [...aliases.fullscreen],
      shortcut: getShortcutKey("fullscreen"),
      shortcutBadge: getShortcutKey("fullscreen"),
      fromMenu: true,
      menuSection: "appearance",
      menuKind: "checkbox",
      checked: opts.fullscreen ?? false,
      visibleInMenu: Boolean(opts.onToggleFullscreen),
      enabled: Boolean(opts.onToggleFullscreen),
      run: () => opts.onToggleFullscreen?.(),
    },

    // --- 3. Undo / Redo (Menu Section: history) ---
    {
      id: "undo",
      name: menu.undo,
      aliases: [...aliases.undo],
      shortcut: getShortcutKey("undo"),
      shortcutBadge: getShortcutKey("undo"),
      fromMenu: true,
      menuSection: "history",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onUndo,
      enabled: hasDoc && Boolean(opts.onUndo),
      disabledReason: !hasDoc ? strings.commands.noDocument : strings.commands.noUndo,
      run: () => opts.onUndo?.(),
    },
    {
      id: "redo",
      name: menu.redo,
      aliases: [...aliases.redo],
      shortcut: getShortcutKey("redo"),
      shortcutBadge: getShortcutKey("redo"),
      fromMenu: true,
      menuSection: "history",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onRedo,
      enabled: hasDoc && Boolean(opts.onRedo),
      disabledReason: !hasDoc ? strings.commands.noDocument : strings.commands.noRedo,
      run: () => opts.onRedo?.(),
    },

    // --- 4. Main Menu Actions (Menu Section: actions) ---
    {
      id: "save",
      name: menu.save,
      aliases: [...aliases.save],
      shortcut: getShortcutKey("save"),
      shortcutBadge: getShortcutKey("save"),
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onSave,
      enabled: hasDoc && Boolean(opts.onSave),
      disabledReason: !hasDoc ? strings.commands.noDocument : strings.commands.noUnsavedChanges,
      run: () => opts.onSave?.(),
    },
    {
      id: "saveAs",
      name: menu.saveAs,
      aliases: [...aliases.saveAs],
      shortcut: getShortcutKey("saveAs"),
      shortcutBadge: getShortcutKey("saveAs"),
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onSaveAs,
      enabled: hasDoc && Boolean(opts.onSaveAs),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onSaveAs?.(),
    },
    {
      id: "createPdf",
      name: menu.createPdf,
      aliases: [...aliases.createPdf],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onCreatePdf,
      enabled: Boolean(opts.onCreatePdf),
      run: () => opts.onCreatePdf?.(),
    },
    {
      id: "ocr",
      name: opts.ocr?.running ? strings.ocr.menuRunning : strings.ocr.menu,
      aliases: [...aliases.ocr],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: Boolean(opts.ocr),
      menuDisabled: Boolean(opts.ocr?.running),
      enabled: hasDoc && Boolean(opts.ocr && !opts.ocr.running),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.ocr?.running
          ? strings.ocr.menuRunning
          : undefined,
      run: () => opts.ocr?.onStart(),
    },
    {
      id: "flatten",
      name: strings.forms.flatten,
      aliases: [...aliases.flatten],
      shortcutBadge: !opts.flatten?.onClick ? strings.permissions.notAllowed : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: Boolean(opts.flatten),
      menuDisabled: !opts.flatten?.onClick,
      enabled: hasDoc && Boolean(opts.flatten?.onClick),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : !opts.flatten?.onClick
          ? strings.permissions.notAllowed
          : undefined,
      run: () => opts.flatten?.onClick?.(),
    },
    {
      id: "export",
      name: menu.export,
      aliases: [...aliases.export],
      shortcutBadge: opts.exportBlocked ? strings.permissions.notAllowed : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onExport,
      enabled: hasDoc && Boolean(opts.onExport && !opts.exportBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.exportBlocked
          ? strings.permissions.notAllowed
          : undefined,
      run: () => opts.onExport?.(),
    },
    {
      id: "exportAnnotations",
      name: menu.exportAnnotations,
      aliases: [...aliases.exportAnnotations],
      shortcutBadge: opts.exportAnnotationsBlocked ? strings.permissions.notAllowed : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onExportAnnotations,
      enabled: hasDoc && Boolean(opts.onExportAnnotations && !opts.exportAnnotationsBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.exportAnnotationsBlocked
          ? strings.permissions.notAllowed
          : undefined,
      run: () => opts.onExportAnnotations?.(),
    },
    {
      id: "compareText",
      name: menu.compareText,
      aliases: [...aliases.compareText],
      shortcutBadge: opts.compareTextBlocked,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onCompareText,
      enabled: hasDoc && Boolean(opts.onCompareText && !opts.compareTextBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.compareTextBlocked,
      run: () => opts.onCompareText?.(),
    },
    {
      id: "compareVisual",
      name: menu.compareVisual,
      aliases: [...aliases.compareVisual],
      shortcutBadge: opts.compareVisualBlocked,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onCompareVisual,
      enabled: hasDoc && Boolean(opts.onCompareVisual && !opts.compareVisualBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.compareVisualBlocked,
      run: () => opts.onCompareVisual?.(),
    },
    {
      id: "privacyExport",
      name: menu.privacyExport,
      aliases: [...aliases.privacyExport],
      shortcutBadge: opts.privacyExportBlocked ? strings.privacyExport.encrypted : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onPrivacyExport,
      enabled: hasDoc && Boolean(opts.onPrivacyExport && !opts.privacyExportBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.privacyExportBlocked
          ? strings.privacyExport.encrypted
          : undefined,
      run: () => opts.onPrivacyExport?.(),
    },
    {
      id: "sanitizeCopy",
      name: menu.sanitizeCopy,
      aliases: [...aliases.sanitizeCopy],
      shortcutBadge: opts.sanitizeCopyBlocked,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onSanitizeCopy,
      enabled: hasDoc && Boolean(opts.onSanitizeCopy && !opts.sanitizeCopyBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.sanitizeCopyBlocked,
      run: () => opts.onSanitizeCopy?.(),
    },
    {
      id: "pageMarks",
      name: menu.pageMarks,
      aliases: [...aliases.pageMarks],
      shortcutBadge: opts.pageMarksBlocked ? strings.pageMarks.encrypted : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onPageMarks,
      enabled: hasDoc && Boolean(opts.onPageMarks && !opts.pageMarksBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.pageMarksBlocked
          ? strings.pageMarks.encrypted
          : undefined,
      run: () => opts.onPageMarks?.(),
    },
    {
      id: "watermark",
      name: menu.watermark,
      aliases: [...aliases.watermark],
      shortcutBadge: opts.watermarkBlocked ? opts.watermarkBlocked : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onWatermark,
      enabled: hasDoc && Boolean(opts.onWatermark && !opts.watermarkBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.watermarkBlocked,
      run: () => opts.onWatermark?.(),
    },
    {
      id: "encryptCopy",
      name: menu.encryptCopy,
      aliases: [...aliases.encryptCopy],
      shortcutBadge: opts.encryptCopyBlocked ? strings.encryption.encrypted : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onEncryptCopy,
      enabled: hasDoc && Boolean(opts.onEncryptCopy && !opts.encryptCopyBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.encryptCopyBlocked
          ? strings.encryption.encrypted
          : undefined,
      run: () => opts.onEncryptCopy?.(),
    },
    {
      id: "decryptCopy",
      name: menu.decryptCopy,
      aliases: [...aliases.decryptCopy],
      shortcutBadge: opts.decryptCopyBlocked ? strings.decryption.unencryptedBlocked : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onDecryptCopy,
      enabled: hasDoc && Boolean(opts.onDecryptCopy && !opts.decryptCopyBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.decryptCopyBlocked
          ? strings.decryption.unencryptedBlocked
          : undefined,
      run: () => opts.onDecryptCopy?.(),
    },
    {
      id: "searchAll",
      name: menu.searchAll,
      aliases: [...aliases.searchAll],
      shortcut: getShortcutKey("searchAll"),
      shortcutBadge: getShortcutKey("searchAll"),
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onSearchAll,
      enabled: Boolean(opts.onSearchAll),
      run: () => opts.onSearchAll?.(),
    },
    {
      id: "batch",
      name: menu.batch,
      aliases: [...aliases.batch],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onBatch,
      enabled: Boolean(opts.onBatch),
      run: () => opts.onBatch?.(),
    },
    {
      id: "forgetPassword",
      name: strings.password.forgetMenu,
      aliases: [...aliases.forgetPassword],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: Boolean(opts.onForgetPassword),
      enabled: hasDoc && Boolean(opts.onForgetPassword),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onForgetPassword?.(),
    },
    {
      id: "compressImages",
      name: menu.compressImages,
      aliases: [...aliases.compressImages],
      shortcutBadge: opts.compressImagesBlocked ? strings.compress.encrypted : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onCompressImages,
      enabled: hasDoc && Boolean(opts.onCompressImages && !opts.compressImagesBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.compressImagesBlocked
          ? strings.compress.encrypted
          : undefined,
      run: () => opts.onCompressImages?.(),
    },
    {
      id: "sign",
      name: menu.sign,
      aliases: [...aliases.sign],
      shortcutBadge: opts.signBlocked ? strings.signing.encryptedShort : undefined,
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onSign,
      enabled: hasDoc && Boolean(opts.onSign && !opts.signBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.signBlocked
          ? strings.signing.encryptedShort
          : undefined,
      run: () => opts.onSign?.(),
    },
    {
      id: "print",
      name: menu.print,
      aliases: [...aliases.print],
      shortcut: getShortcutKey("print"),
      shortcutBadge: opts.printBlocked ? strings.permissions.notAllowed : "Ctrl+P",
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onPrint,
      enabled: hasDoc && Boolean(opts.onPrint && !opts.printBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.printBlocked
          ? strings.permissions.notAllowed
          : undefined,
      run: () => opts.onPrint?.(),
    },
    {
      id: "documentProperties",
      name: menu.documentProperties,
      aliases: [...aliases.documentProperties],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      menuDisabled: !opts.onDocumentProperties,
      enabled: hasDoc && Boolean(opts.onDocumentProperties),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onDocumentProperties?.(),
    },
    {
      id: "sensitive",
      name: opts.sensitive?.marked ? menu.unmarkSensitive : menu.markSensitive,
      aliases: [...aliases.sensitive],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: Boolean(opts.sensitive),
      enabled: hasDoc && Boolean(opts.sensitive?.onClick),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.sensitive?.onClick(),
    },
    {
      id: "dontRecord",
      name: menu.dontRecord,
      aliases: [...aliases.dontRecord],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "checkbox",
      checked: opts.recording?.recorded === false,
      visibleInMenu: Boolean(opts.recording),
      enabled: hasDoc && Boolean(opts.recording && opts.recording.recorded !== null),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => {
        if (opts.recording && opts.recording.recorded !== null) {
          opts.recording.onChange(!opts.recording.recorded);
        }
      },
    },
    {
      id: "shortcuts",
      name: menu.shortcuts,
      aliases: [...aliases.shortcuts],
      shortcut: getShortcutKey("help"),
      shortcutBadge: getShortcutKey("help"),
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      enabled: Boolean(opts.onShowShortcuts),
      run: () => opts.onShowShortcuts(),
    },
    {
      id: "settings",
      name: menu.settings,
      aliases: [...aliases.settings],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: Boolean(opts.onShowSettings),
      enabled: Boolean(opts.onShowSettings),
      run: () => opts.onShowSettings?.(),
    },
    {
      id: "about",
      name: menu.about,
      aliases: [...aliases.about],
      fromMenu: true,
      menuSection: "actions",
      menuKind: "item",
      visibleInMenu: true,
      enabled: Boolean(opts.onShowAbout),
      run: () => opts.onShowAbout(),
    },

    // --- 5. Set Default (Menu Section: default) ---
    {
      id: "setDefault",
      name: menu.setDefault,
      aliases: [...aliases.setDefault],
      fromMenu: true,
      menuSection: "default",
      menuKind: "item",
      visibleInMenu: true,
      enabled: Boolean(opts.onSetDefault),
      run: () => opts.onSetDefault(),
    },

    // --- 6. Shortcut-only / Toolbar Commands (25 items) ---
    {
      id: "open",
      name: strings.shortcuts.descriptions.open,
      aliases: [...aliases.open],
      shortcut: getShortcutKey("open"),
      fromMenu: false,
      enabled: Boolean(opts.onOpen),
      run: () => opts.onOpen(),
    },
    {
      id: "close",
      name: strings.shortcuts.descriptions.close,
      aliases: [...aliases.close],
      shortcut: getShortcutKey("close"),
      fromMenu: false,
      enabled: Boolean(opts.onClose),
      run: () => opts.onClose?.(),
    },
    {
      id: "nextTab",
      name: strings.shortcuts.descriptions.nextTab,
      aliases: [...aliases.nextTab],
      shortcut: getShortcutKey("nextTab"),
      fromMenu: false,
      enabled: Boolean(opts.onNextTab),
      run: () => opts.onNextTab?.(),
    },
    {
      id: "previousTab",
      name: strings.shortcuts.descriptions.previousTab,
      aliases: [...aliases.previousTab],
      shortcut: getShortcutKey("previousTab"),
      fromMenu: false,
      enabled: Boolean(opts.onPreviousTab),
      run: () => opts.onPreviousTab?.(),
    },
    {
      id: "copy",
      name: strings.shortcuts.descriptions.copy,
      aliases: [...aliases.copy],
      shortcut: getShortcutKey("copy"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onCopy),
      disabledReason: !hasDoc ? strings.commands.noDocument : strings.commands.noSelection,
      run: () => opts.onCopy?.(),
    },
    {
      id: "search",
      name: strings.shortcuts.descriptions.search,
      aliases: [...aliases.search],
      shortcut: getShortcutKey("search"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onSearch),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onSearch(),
    },
    {
      id: "findNext",
      name: strings.shortcuts.descriptions.findNext,
      aliases: [...aliases.findNext],
      shortcut: getShortcutKey("findNext"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onFindNext),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onFindNext?.(),
    },
    {
      id: "findPrevious",
      name: strings.shortcuts.descriptions.findPrevious,
      aliases: [...aliases.findPrevious],
      shortcut: getShortcutKey("findPrevious"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onFindPrevious),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onFindPrevious?.(),
    },
    {
      id: "zoomIn",
      name: strings.shortcuts.descriptions.zoomIn,
      aliases: [...aliases.zoomIn],
      shortcut: getShortcutKey("zoomIn"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onZoomIn),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onZoomIn(),
    },
    {
      id: "zoomOut",
      name: strings.shortcuts.descriptions.zoomOut,
      aliases: [...aliases.zoomOut],
      shortcut: getShortcutKey("zoomOut"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onZoomOut),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onZoomOut(),
    },
    {
      id: "fitPage",
      name: strings.shortcuts.descriptions.fitPage,
      aliases: [...aliases.fitPage],
      shortcut: getShortcutKey("fitPage"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onFitPage || opts.onZoomChange),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => {
        if (opts.onFitPage) opts.onFitPage();
        else opts.onZoomChange?.("fitPage");
      },
    },
    {
      id: "actualSize",
      name: strings.shortcuts.descriptions.actualSize,
      aliases: [...aliases.actualSize],
      shortcut: getShortcutKey("actualSize"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onActualSize || opts.onZoomChange),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => {
        if (opts.onActualSize) opts.onActualSize();
        else opts.onZoomChange?.(100);
      },
    },
    {
      id: "fitWidth",
      name: strings.shortcuts.descriptions.fitWidth,
      aliases: [...aliases.fitWidth],
      shortcut: getShortcutKey("fitWidth"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onFitWidth || opts.onZoomChange),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => {
        if (opts.onFitWidth) opts.onFitWidth();
        else opts.onZoomChange?.("fitWidth");
      },
    },
    {
      id: "rotateCw",
      name: strings.shortcuts.descriptions.rotateCw,
      aliases: [...aliases.rotateCw],
      shortcut: getShortcutKey("rotateCw"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onRotate),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onRotate(1),
    },
    {
      id: "rotateCcw",
      name: strings.shortcuts.descriptions.rotateCcw,
      aliases: [...aliases.rotateCcw],
      shortcut: getShortcutKey("rotateCcw"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onRotate),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onRotate(-1),
    },
    {
      id: "goToPage",
      name: strings.shortcuts.descriptions.goToPage,
      aliases: [...aliases.goToPage],
      shortcut: getShortcutKey("goToPage"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onGoToPage || opts.pageInputRef),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => {
        if (opts.pageInputRef && typeof opts.pageInputRef === "object" && "current" in opts.pageInputRef) {
          opts.pageInputRef.current?.focus();
        }
      },
    },
    {
      id: "firstPage",
      name: strings.shortcuts.descriptions.firstPage,
      aliases: [...aliases.firstPage],
      shortcut: getShortcutKey("firstPage"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onFirstPage || opts.onGoToPage),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => {
        if (opts.onFirstPage) opts.onFirstPage();
        else opts.onGoToPage?.(1);
      },
    },
    {
      id: "lastPage",
      name: strings.shortcuts.descriptions.lastPage,
      aliases: [...aliases.lastPage],
      shortcut: getShortcutKey("lastPage"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onLastPage || (opts.document && opts.onGoToPage)),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => {
        if (opts.onLastPage) opts.onLastPage();
        else if (opts.document && opts.onGoToPage) opts.onGoToPage(opts.document.pageCount);
      },
    },
    {
      id: "historyBack",
      name: strings.shortcuts.descriptions.historyBack,
      aliases: [...aliases.historyBack],
      shortcut: getShortcutKey("historyBack"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.canGoBack && opts.onGoBack),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : !opts.canGoBack
          ? strings.commands.noHistory
          : undefined,
      run: () => opts.onGoBack?.(),
    },
    {
      id: "historyForward",
      name: strings.shortcuts.descriptions.historyForward,
      aliases: [...aliases.historyForward],
      shortcut: getShortcutKey("historyForward"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.canGoForward && opts.onGoForward),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : !opts.canGoForward
          ? strings.commands.noHistory
          : undefined,
      run: () => opts.onGoForward?.(),
    },
    {
      id: "toggleSidebar",
      name: strings.shortcuts.descriptions.toggleSidebar,
      aliases: [...aliases.toggleSidebar],
      shortcut: getShortcutKey("toggleSidebar"),
      fromMenu: false,
      enabled: Boolean(opts.onToggleSidebar),
      run: () => opts.onToggleSidebar(),
    },
    {
      id: "nextRegion",
      name: strings.shortcuts.descriptions.nextRegion,
      aliases: [...aliases.nextRegion],
      shortcut: getShortcutKey("nextRegion"),
      fromMenu: false,
      enabled: Boolean(opts.onNextRegion),
      run: () => opts.onNextRegion?.(),
    },
    {
      id: "previousRegion",
      name: strings.shortcuts.descriptions.previousRegion,
      aliases: [...aliases.previousRegion],
      shortcut: getShortcutKey("previousRegion"),
      fromMenu: false,
      enabled: Boolean(opts.onPreviousRegion),
      run: () => opts.onPreviousRegion?.(),
    },
    {
      id: "presentation",
      name: strings.shortcuts.descriptions.presentation,
      aliases: [...aliases.presentation],
      shortcut: getShortcutKey("presentation"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onPresentation),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onPresentation?.(),
    },
    {
      id: "presentationCurrent",
      name: strings.shortcuts.descriptions.presentationCurrent,
      aliases: [...aliases.presentationCurrent],
      shortcut: getShortcutKey("presentationCurrent"),
      fromMenu: false,
      enabled: hasDoc && Boolean(opts.onPresentationCurrent),
      disabledReason: !hasDoc ? strings.commands.noDocument : undefined,
      run: () => opts.onPresentationCurrent?.(),
    },

    // --- 7. Palette-only commands ---
    {
      id: "insertFileAfterCurrent",
      name: strings.commands.insertFileAfterCurrent,
      aliases: [...aliases.insertFileAfterCurrent],
      fromMenu: false,
      visibleInMenu: false,
      enabled: hasDoc && Boolean(opts.onInsertFrom && !opts.insertFromBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.insertFromBlocked
          ? strings.permissions.notAllowed
          : undefined,
      run: () => {
        if (opts.document && opts.onInsertFrom) {
          // The palette has no message area of its own: a refusal goes to the status bar (#584).
          void Promise.resolve(opts.onInsertFrom(opts.document.currentPage)).catch((error: unknown) => {
            opts.onHint?.(insertFailureMessage(error));
          });
        }
      },
    },
    {
      id: "replaceCurrentPage",
      name: strings.commands.replaceCurrentPage,
      aliases: [...aliases.replaceCurrentPage],
      fromMenu: false,
      visibleInMenu: false,
      enabled: hasDoc && Boolean(opts.onReplacePages && !opts.insertFromBlocked),
      disabledReason: !hasDoc
        ? strings.commands.noDocument
        : opts.insertFromBlocked
          ? strings.permissions.notAllowed
          : undefined,
      run: () => {
        if (opts.document && opts.onReplacePages) {
          void Promise.resolve(opts.onReplacePages(opts.document.currentPage - 1, 1)).catch((error: unknown) => {
            opts.onHint?.(insertFailureMessage(error));
          });
        }
      },
    },
  ];

  return commands;
}

/**
 * Filters and sorts commands:
 * - Substring matching against command name and aliases (case-insensitive for Latin letters).
 * - Recently executed commands sorted to the front according to recentIds.
 */
export function filterCommands(
  commands: Command[],
  query: string,
  recentIds: string[] = recentCommandIds,
): Command[] {
  const q = query.trim().toLowerCase();

  const matching = q === ""
    ? commands
    : commands.filter((cmd) => {
        if (cmd.name.toLowerCase().includes(q)) return true;
        return cmd.aliases.some((alias) => alias.toLowerCase().includes(q));
      });

  return [...matching].sort((a, b) => {
    const aRecentIdx = recentIds.indexOf(a.id);
    const bRecentIdx = recentIds.indexOf(b.id);

    if (aRecentIdx !== -1 && bRecentIdx !== -1) {
      return aRecentIdx - bRecentIdx;
    }
    if (aRecentIdx !== -1) return -1;
    if (bRecentIdx !== -1) return 1;

    return 0;
  });
}
