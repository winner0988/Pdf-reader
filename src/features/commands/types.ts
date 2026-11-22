import type { ToolbarProps } from "@/features/shell/Toolbar";

export type CommandId =
  // Menu items (37):
  | "layoutSingle"
  | "layoutTwoPage"
  | "layoutTwoPageCover"
  | "themeSystem"
  | "themeLight"
  | "themeDark"
  | "fullscreen"
  | "undo"
  | "redo"
  | "save"
  | "saveAs"
  | "createPdf"
  | "ocr"
  | "flatten"
  | "export"
  | "exportAnnotations"
  | "exportXfdf"
  | "compareText"
  | "compareVisual"
  | "privacyExport"
  | "sanitizeCopy"
  | "pageMarks"
  | "watermark"
  | "encryptCopy"
  | "decryptCopy"
  | "searchAll"
  | "batch"
  | "forgetPassword"
  | "compressImages"
  | "sign"
  | "print"
  | "documentProperties"
  | "sensitive"
  | "dontRecord"
  | "shortcuts"
  | "settings"
  | "about"
  | "setDefault"
  // Shortcut-only items (25):
  | "open"
  | "close"
  | "nextTab"
  | "previousTab"
  | "copy"
  | "search"
  | "findNext"
  | "findPrevious"
  | "zoomIn"
  | "zoomOut"
  | "fitPage"
  | "actualSize"
  | "fitWidth"
  | "rotateCw"
  | "rotateCcw"
  | "goToPage"
  | "firstPage"
  | "lastPage"
  | "historyBack"
  | "historyForward"
  | "toggleSidebar"
  | "nextRegion"
  | "previousRegion"
  | "presentation"
  | "presentationCurrent"
  // Palette-only items:
  | "insertFileAfterCurrent"
  | "replaceCurrentPage";

export type MenuSection = "layout" | "appearance" | "history" | "actions" | "default";
export type MenuItemKind = "item" | "radio" | "checkbox";

export type Command = {
  id: CommandId;
  name: string;
  aliases: string[];
  shortcut?: string;
  enabled: boolean;
  disabledReason?: string;
  run: () => void;
  // Metadata for menu rendering
  fromMenu: boolean;
  menuSection?: MenuSection;
  menuKind?: MenuItemKind;
  menuLabel?: string;
  menuDisabled?: boolean;
  shortcutBadge?: string;
  checked?: boolean;
  visibleInMenu?: boolean;
};

export type CommandOptions = ToolbarProps & {
  onClose?: () => void;
  onCopy?: () => void;
  onFindNext?: () => void;
  onFindPrevious?: () => void;
  onFitPage?: () => void;
  onActualSize?: () => void;
  onFitWidth?: () => void;
  onFirstPage?: () => void;
  onLastPage?: () => void;
  onPresentation?: () => void;
  onPresentationCurrent?: () => void;
  onNextRegion?: () => void;
  onPreviousRegion?: () => void;
  onNextTab?: () => void;
  onPreviousTab?: () => void;
  onInsertFrom?: (at: number) => Promise<number | null> | void;
  onReplacePages?: (first: number, count: number) => Promise<number | null> | void;
  insertFromBlocked?: boolean;
  onDecryptCopy?: () => void;
  decryptCopyBlocked?: boolean;
};
