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
/// The dialog for the file whose pages are put into the document (B2-06).
pub const PAGES_SOURCE_DIALOG_TITLE: &str = "選擇要插入頁面的 PDF 檔案";

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

/// Encrypting a copy (B2-15): where the copy goes, what it is called at first, and why the
/// document's own file cannot be chosen.
pub const ENCRYPTED_COPY_DIALOG_TITLE: &str = "加密並另存新檔：選擇副本的位置";
pub fn encrypted_copy_file_name(stem: &str) -> String {
    format!("{stem}（已加密）.pdf")
}
pub const ENCRYPTED_COPY_SAME_FILE_TITLE: &str = "請選擇其他檔案";
pub const ENCRYPTED_COPY_SAME_FILE_MESSAGE: &str =
    "加密會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。";

/// Importing a language for recognising the text of scanned pages (B2-10).
pub const LANGUAGE_DIALOG_TITLE: &str = "匯入 OCR 語言資料";
pub const LANGUAGE_FILTER_NAME: &str = "Tesseract 語言資料（.traineddata）";

/// Export (B2-04): where the text goes, and the folder the page images go to.
pub const EXPORT_TEXT_DIALOG_TITLE: &str = "匯出純文字";
pub const TEXT_FILTER_NAME: &str = "純文字檔";
pub const EXPORT_IMAGES_DIALOG_TITLE: &str = "選擇匯出頁面圖片的資料夾";
/// The dialog for the picture of a custom stamp (B2-08).
pub const STAMP_IMAGE_DIALOG_TITLE: &str = "選擇印章要用的圖片";
pub const IMAGE_FILTER_NAME: &str = "圖片（PNG、JPEG）";
pub const SPLIT_DIALOG_TITLE: &str = "將選取的頁面另存為新檔";
pub const SPLIT_SAME_FILE_TITLE: &str = "請選擇其他檔案";
pub const SPLIT_SAME_FILE_MESSAGE: &str =
    "拆分出的頁面是一份新的檔案，不會取代原本的檔案。請選擇原檔以外的位置或檔名。";
pub const SPLIT_FOLDER_DIALOG_TITLE: &str = "選擇存放拆分後檔案的資料夾";
/// The name suggested for a file of some pages of a document (B2-06): the pages' numbers when
/// they follow one another, else only that they were chosen.
pub fn split_file_name(stem: &str, pages: &[u32]) -> String {
    match (pages.first(), pages.last()) {
        (Some(first), Some(last)) if follow_one_another(pages) => {
            let (first, last) = (first.saturating_add(1), last.saturating_add(1));
            if first == last {
                format!("{stem}-p{first}.pdf")
            } else {
                format!("{stem}-p{first}-{last}.pdf")
            }
        }
        _ => format!("{stem}（選取的頁面）.pdf"),
    }
}

/// The name of part number `part` (0-based) of `parts` that a document is split into (B2-06): the
/// pages' numbers when they follow one another, else the number of the part.
pub fn split_part_name(stem: &str, pages: &[u32], part: usize, parts: usize) -> String {
    if follow_one_another(pages) {
        split_file_name(stem, pages)
    } else {
        let width = parts.to_string().len();
        format!("{stem}-{:0width$}.pdf", part + 1)
    }
}

fn follow_one_another(pages: &[u32]) -> bool {
    pages
        .windows(2)
        .all(|pair| pair[0].checked_add(1) == Some(pair[1]))
}
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
    use super::{split_file_name, split_part_name, window_title};

    #[test]
    fn split_files_are_named_by_their_pages_or_by_their_number() {
        assert_eq!(split_file_name("報告", &[1, 2, 3]), "報告-p2-4.pdf");
        assert_eq!(split_file_name("報告", &[4]), "報告-p5.pdf");
        assert_eq!(split_file_name("報告", &[1, 3]), "報告（選取的頁面）.pdf");
        assert_eq!(split_file_name("報告", &[2, 1]), "報告（選取的頁面）.pdf");
        assert_eq!(split_part_name("報告", &[0, 1], 0, 3), "報告-p1-2.pdf");
        // Pages that do not follow one another: the number of the part, as wide as the last.
        assert_eq!(split_part_name("報告", &[0, 2], 1, 12), "報告-02.pdf");
        assert_eq!(split_part_name("報告", &[0, 2], 1, 5), "報告-2.pdf");
    }

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
