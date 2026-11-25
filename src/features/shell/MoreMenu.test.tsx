import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { TooltipProvider } from "@/components/ui/tooltip";
import { Toolbar, type ToolbarProps } from "@/features/shell/Toolbar";
import { strings } from "@/i18n";

function renderToolbar(props: Partial<ToolbarProps> = {}) {
  const user = userEvent.setup();
  const defaultProps: ToolbarProps = {
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
    ...props,
  };

  const utils = render(
    <TooltipProvider>
      <Toolbar {...defaultProps} />
    </TooltipProvider>,
  );

  return { ...utils, user };
}

describe("MoreMenu structure and order", () => {
  it("renders all items in exact order and disabled states with full props", async () => {
    const { user } = renderToolbar({
      layoutMode: "single",
      onLayoutModeChange: vi.fn(),
      fullscreen: false,
      onToggleFullscreen: vi.fn(),
      onUndo: vi.fn(),
      onRedo: vi.fn(),
      onSave: vi.fn(),
      onSaveAs: vi.fn(),
      onCreatePdf: vi.fn(),
      ocr: { running: false, onStart: vi.fn() },
      onSearchableCopy: vi.fn(),
      flatten: { onClick: vi.fn() },
      onExport: vi.fn(),
      onExportAnnotations: vi.fn(),
      onExportXfdf: vi.fn(),
      onCompareText: vi.fn(),
      onCompareVisual: vi.fn(),
      onPrivacyExport: vi.fn(),
      onSanitizeCopy: vi.fn(),
      onPageMarks: vi.fn(),
      onWatermark: vi.fn(),
      onEncryptCopy: vi.fn(),
      onDecryptCopy: vi.fn(),
      onSearchAll: vi.fn(),
      onBatch: vi.fn(),
      onForgetPassword: vi.fn(),
      onCompressImages: vi.fn(),
      onSign: vi.fn(),
      onPrint: vi.fn(),
      onDocumentProperties: vi.fn(),
      sensitive: { marked: false, onClick: vi.fn() },
      recording: { recorded: true, onChange: vi.fn() },
      onShowSettings: vi.fn(),
    });

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));

    const menu = await screen.findByRole("menu");
    const items = Array.from(
      menu.querySelectorAll<HTMLElement>(
        '[role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"]',
      ),
    );

    const snapshot = items.map((el) => ({
      text: el.textContent?.trim(),
      role: el.getAttribute("role"),
      disabled: el.hasAttribute("data-disabled") || el.hasAttribute("disabled") || el.getAttribute("aria-disabled") === "true",
    }));

    expect(snapshot).toEqual([
      { text: "單頁", role: "menuitemradio", disabled: false },
      { text: "雙頁", role: "menuitemradio", disabled: false },
      { text: "雙頁（封面單獨）", role: "menuitemradio", disabled: false },
      { text: "跟隨系統", role: "menuitemradio", disabled: false },
      { text: "淺色", role: "menuitemradio", disabled: false },
      { text: "深色", role: "menuitemradio", disabled: false },
      { text: "全螢幕F11", role: "menuitemcheckbox", disabled: false },
      { text: "復原Ctrl+Z", role: "menuitem", disabled: false },
      { text: "重做Ctrl+Y", role: "menuitem", disabled: false },
      { text: "儲存Ctrl+S", role: "menuitem", disabled: false },
      { text: "另存新檔…Ctrl+Shift+S", role: "menuitem", disabled: false },
      { text: "從檔案建立 PDF…", role: "menuitem", disabled: false },
      { text: "辨識此文件的文字", role: "menuitem", disabled: false },
      { text: "另存為可搜尋的 PDF…", role: "menuitem", disabled: false },
      { text: "扁平化表單…", role: "menuitem", disabled: false },
      { text: "匯出…", role: "menuitem", disabled: false },
      { text: "匯出註解摘要…", role: "menuitem", disabled: false },
      { text: "匯出註解（XFDF）…", role: "menuitem", disabled: false },
      { text: "與另一個分頁比較文字…", role: "menuitem", disabled: false },
      { text: "與另一個分頁比較畫面…", role: "menuitem", disabled: false },
      { text: "隱私匯出…", role: "menuitem", disabled: false },
      { text: "分享前淨化…", role: "menuitem", disabled: false },
      { text: "頁碼與 Bates 編號…", role: "menuitem", disabled: false },
      { text: "浮水印…", role: "menuitem", disabled: false },
      { text: "加密並另存新檔…", role: "menuitem", disabled: false },
      { text: "另存沒有密碼的副本…", role: "menuitem", disabled: false },
      { text: "在所有文件中搜尋…Ctrl+Shift+F", role: "menuitem", disabled: false },
      { text: "批次處理…", role: "menuitem", disabled: false },
      { text: "忘記這份文件的密碼", role: "menuitem", disabled: false },
      { text: "壓縮圖片並另存新檔…", role: "menuitem", disabled: false },
      { text: "簽署…", role: "menuitem", disabled: false },
      { text: "列印…Ctrl+P", role: "menuitem", disabled: false },
      { text: "文件內容…", role: "menuitem", disabled: false },
      { text: "標記為機敏…", role: "menuitem", disabled: false },
      { text: "不記錄此檔案", role: "menuitemcheckbox", disabled: false },
      { text: "快捷鍵Ctrl+/", role: "menuitem", disabled: false },
      { text: "設定…", role: "menuitem", disabled: false },
      { text: "關於", role: "menuitem", disabled: false },
      { text: "設為預設 PDF 閱讀器", role: "menuitem", disabled: false },
    ]);
  });

  it("renders blocked and disabled states accurately", async () => {
    const { user } = renderToolbar({
      // No callbacks for save, undo, redo, etc.
      printBlocked: true,
      onPrint: vi.fn(),
      exportBlocked: true,
      onExport: vi.fn(),
      exportAnnotationsBlocked: true,
      onExportAnnotations: vi.fn(),
      exportXfdfBlocked: true,
      onExportXfdf: vi.fn(),
      compareTextBlocked: strings.compare.onlyOneTab,
      onCompareText: vi.fn(),
      compareVisualBlocked: strings.compare.onlyOneTab,
      onCompareVisual: vi.fn(),
      privacyExportBlocked: true,
      onPrivacyExport: vi.fn(),
      sanitizeCopyBlocked: strings.sanitize.encrypted,
      onSanitizeCopy: vi.fn(),
      pageMarksBlocked: true,
      onPageMarks: vi.fn(),
      encryptCopyBlocked: true,
      onEncryptCopy: vi.fn(),
      compressImagesBlocked: true,
      onCompressImages: vi.fn(),
      signBlocked: true,
      onSign: vi.fn(),
      flatten: { onClick: undefined },
      ocr: { running: true, onStart: vi.fn() },
    });

    await user.click(screen.getByRole("button", { name: strings.toolbar.more }));

    const menu = await screen.findByRole("menu");
    const items = Array.from(
      menu.querySelectorAll<HTMLElement>(
        '[role="menuitem"], [role="menuitemradio"], [role="menuitemcheckbox"]',
      ),
    );

    const snapshot = items.map((el) => ({
      text: el.textContent?.trim(),
      role: el.getAttribute("role"),
      disabled: el.hasAttribute("data-disabled") || el.hasAttribute("disabled") || el.getAttribute("aria-disabled") === "true",
    }));

    expect(snapshot).toEqual([
      { text: "跟隨系統", role: "menuitemradio", disabled: false },
      { text: "淺色", role: "menuitemradio", disabled: false },
      { text: "深色", role: "menuitemradio", disabled: false },
      { text: "復原Ctrl+Z", role: "menuitem", disabled: true },
      { text: "重做Ctrl+Y", role: "menuitem", disabled: true },
      { text: "儲存Ctrl+S", role: "menuitem", disabled: true },
      { text: "另存新檔…Ctrl+Shift+S", role: "menuitem", disabled: true },
      { text: "從檔案建立 PDF…", role: "menuitem", disabled: true },
      { text: "正在辨識文字…", role: "menuitem", disabled: true },
      { text: "另存為可搜尋的 PDF…", role: "menuitem", disabled: true },
      { text: `扁平化表單…${strings.permissions.notAllowed}`, role: "menuitem", disabled: true },
      { text: `匯出…${strings.permissions.notAllowed}`, role: "menuitem", disabled: false }, // onExport provided
      { text: `匯出註解摘要…${strings.permissions.notAllowed}`, role: "menuitem", disabled: false }, // onExportAnnotations provided
      { text: `匯出註解（XFDF）…${strings.permissions.notAllowed}`, role: "menuitem", disabled: false }, // onExportXfdf provided
      { text: `與另一個分頁比較文字…${strings.compare.onlyOneTab}`, role: "menuitem", disabled: false }, // onCompareText provided
      { text: `與另一個分頁比較畫面…${strings.compare.onlyOneTab}`, role: "menuitem", disabled: false }, // onCompareVisual provided
      { text: `隱私匯出…${strings.privacyExport.encrypted}`, role: "menuitem", disabled: false },
      { text: `分享前淨化…${strings.sanitize.encrypted}`, role: "menuitem", disabled: false },
      { text: `頁碼與 Bates 編號…${strings.pageMarks.encrypted}`, role: "menuitem", disabled: false },
      { text: "浮水印…", role: "menuitem", disabled: true },
      { text: `加密並另存新檔…${strings.encryption.encrypted}`, role: "menuitem", disabled: false },
      { text: "另存沒有密碼的副本…", role: "menuitem", disabled: true },
      { text: "在所有文件中搜尋…Ctrl+Shift+F", role: "menuitem", disabled: true },
      { text: "批次處理…", role: "menuitem", disabled: true },
      { text: `壓縮圖片並另存新檔…${strings.compress.encrypted}`, role: "menuitem", disabled: false },
      { text: `簽署…${strings.signing.encryptedShort}`, role: "menuitem", disabled: false },
      { text: `列印…${strings.permissions.notAllowed}`, role: "menuitem", disabled: false },
      { text: "文件內容…", role: "menuitem", disabled: true },
      { text: "快捷鍵Ctrl+/", role: "menuitem", disabled: false },
      { text: "關於", role: "menuitem", disabled: false },
      { text: "設為預設 PDF 閱讀器", role: "menuitem", disabled: false },
    ]);
  });
});
