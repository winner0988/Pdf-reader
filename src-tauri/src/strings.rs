//! User-visible text shown by the main process itself (native dialogs). Keep in sync with the
//! string table in docs/ux/screen-map.md, section 8; the WebView's text is in src/i18n/.

pub const OPEN_DIALOG_TITLE: &str = "開啟 PDF 檔案";

/// The window title: the open document's file name, so windows and taskbar buttons can be told
/// apart (REL-03), then the app's name. A dot marks unsaved changes (B2-02), as on the tab.
pub fn window_title(file_name: Option<&str>, unsaved: bool) -> String {
    match (file_name, unsaved) {
        (Some(name), true) => format!("• {name} — PDF Reader"),
        (Some(name), false) => format!("{name} — PDF Reader"),
        (None, _) => "PDF Reader".to_owned(),
    }
}
pub const PDF_FILTER_NAME: &str = "PDF 檔案";

/// Saving a document as another file (B2-02).
pub const SAVE_AS_DIALOG_TITLE: &str = "另存新檔";

/// The privacy export (B2-03): where the copy goes, what it is called at first, and why the
/// document's own file cannot be chosen.
pub const PRIVACY_EXPORT_DIALOG_TITLE: &str = "隱私匯出：選擇副本的位置";
pub fn privacy_export_file_name(stem: &str) -> String {
    format!("{stem}（隱私匯出）.pdf")
}
pub const PRIVACY_EXPORT_SAME_FILE_TITLE: &str = "請選擇其他檔案";
pub const PRIVACY_EXPORT_SAME_FILE_MESSAGE: &str =
    "隱私匯出會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。";

/// Export (B2-04): where the text goes, and the folder the page images go to.
pub const EXPORT_TEXT_DIALOG_TITLE: &str = "匯出純文字";
pub const TEXT_FILTER_NAME: &str = "純文字檔";
pub const EXPORT_IMAGES_DIALOG_TITLE: &str = "選擇匯出頁面圖片的資料夾";
pub const OVERWRITE_TITLE: &str = "檔案已經存在";

/// Asked before exported page images replace files already in the chosen folder.
pub fn overwrite_message(count: usize) -> String {
    format!("這個資料夾已經有 {count} 個同名的檔案。要覆寫嗎？")
}

/// What an exported text file says for a page without text.
pub const NO_TEXT_LAYER_PAGE: &str = "（此頁沒有文字層）";

/// Shown instead of the window when the Microsoft Edge WebView2 Runtime is missing: the
/// installer never downloads it (REL-02).
pub const WEBVIEW2_MISSING_TITLE: &str = "無法開啟 PDF Reader";
pub const WEBVIEW2_MISSING_MESSAGE: &str = "這台電腦缺少 Microsoft Edge WebView2 Runtime，PDF Reader 需要它才能顯示畫面。\n\n\
Windows 11 已內建 WebView2。如果它被移除了，請到 Microsoft 官方網站下載並安裝「WebView2 Runtime」，然後再開啟 PDF Reader：\n\
https://developer.microsoft.com/microsoft-edge/webview2/\n\n\
PDF Reader 不會自行下載任何東西。";

#[cfg(test)]
mod tests {
    use super::window_title;

    #[test]
    fn the_window_title_names_the_open_file_and_marks_unsaved_changes() {
        assert_eq!(
            window_title(Some("report.pdf"), false),
            "report.pdf — PDF Reader"
        );
        assert_eq!(
            window_title(Some("report.pdf"), true),
            "• report.pdf — PDF Reader"
        );
        assert_eq!(window_title(None, false), "PDF Reader");
    }
}
