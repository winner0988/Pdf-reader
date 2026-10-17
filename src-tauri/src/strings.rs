//! User-visible text shown by the main process itself (native dialogs). Keep in sync with the
//! string table in docs/ux/screen-map.md, section 8; the WebView's text is in src/i18n/.

use ipc_contract::types::{BatchOperation, BatchOutcome, ErrorCode};

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

/// Compressing the pictures of a copy (B3-01): where the copy goes, what it is called at first,
/// and why the document's own file cannot be chosen.
pub const COMPRESSED_COPY_DIALOG_TITLE: &str = "壓縮圖片並另存新檔：選擇副本的位置";
pub fn compressed_copy_file_name(stem: &str) -> String {
    format!("{stem}（已壓縮）.pdf")
}

/// The name of a batch's copy whose pictures' metadata was taken out (B3-04).
pub fn exif_cleaned_file_name(stem: &str) -> String {
    format!("{stem}（已清除 EXIF）.pdf")
}
pub const COMPRESSED_COPY_SAME_FILE_TITLE: &str = "請選擇其他檔案";
pub const COMPRESSED_COPY_SAME_FILE_MESSAGE: &str =
    "壓縮會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。";

/// Making a PDF of files (B3-03): the dialog for the files and its filters, the one for where the
/// PDF goes and what it is called at first, and why the PDF cannot go over one of the files.
pub const CREATE_SOURCES_DIALOG_TITLE: &str = "選擇要建立 PDF 的檔案";
pub const CREATE_ALL_FILTER_NAME: &str = "所有可以建立 PDF 的檔案";
pub const CREATE_IMAGES_FILTER_NAME: &str = "圖片（PNG、JPEG、GIF、BMP、TIFF）";
pub const CREATE_HTML_FILTER_NAME: &str = "網頁（HTML）";
pub const CREATE_TEXT_FILTER_NAME: &str = "純文字（TXT）";
pub const CREATE_BOOKS_FILTER_NAME: &str = "電子書與固定版面（EPUB、XPS）";
pub const CREATE_SVG_FILTER_NAME: &str = "SVG 向量圖";
pub const CREATE_OFFICE_FILTER_NAME: &str = "Office 文件（DOCX、XLSX、PPTX；實驗性）";
pub const CREATE_PDF_DIALOG_TITLE: &str = "建立 PDF：選擇 PDF 的位置";
pub fn created_pdf_file_name(stem: &str) -> String {
    format!("{stem}.pdf")
}
pub const CREATE_PDF_SAME_FILE_TITLE: &str = "請選擇其他檔案";
pub const CREATE_PDF_SAME_FILE_MESSAGE: &str =
    "建立 PDF 不會改動原本的檔案。請選擇原檔以外的位置或檔名。";

/// Importing a language for recognising the text of scanned pages (B2-10).
pub const LANGUAGE_DIALOG_TITLE: &str = "匯入 OCR 語言資料";
pub const LANGUAGE_FILTER_NAME: &str = "Tesseract 語言資料（.traineddata）";

/// Export (B2-04): where the text goes, and the folder the page images go to.
pub const EXPORT_TEXT_DIALOG_TITLE: &str = "匯出純文字";
/// The Word document (B3-02): where it goes.
pub const EXPORT_DOCX_DIALOG_TITLE: &str = "匯出 Word 文件";
pub const DOCX_FILTER_NAME: &str = "Word 文件（.docx）";
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
/// A batch (B3-04): the files it is done to, and the folder its copies go to.
pub const BATCH_FILES_DIALOG_TITLE: &str = "選擇要批次處理的 PDF 檔案";
pub const BATCH_FOLDER_DIALOG_TITLE: &str = "選擇存放批次處理結果的資料夾";

/// A batch with the window closed (B3-04, ADR 0005): the notification area icon (it says what
/// it is and that it ends with the batch), its menu, the report in the batch's folder and the
/// message once the batch is done.
pub fn batch_tray_tooltip(done: u32, total: u32) -> String {
    format!("PDF Reader：批次處理中（{done}／{total}），完成後就會結束")
}
pub const BATCH_TRAY_SHOW: &str = "顯示視窗";
pub const BATCH_TRAY_STOP: &str = "停止批次處理";
pub const BATCH_DONE_TITLE: &str = "批次處理完成";
pub fn batch_done_message(summary: &str, report: Option<&str>) -> String {
    match report {
        Some(path) => format!("{summary}\n\n每個檔案的結果寫在：\n{path}"),
        None => format!("{summary}\n\n報告無法寫進資料夾。"),
    }
}
pub const BATCH_REPORT_FILE_NAME: &str = "批次處理報告.txt";
pub const BATCH_REPORT_TITLE: &str = "PDF Reader 批次處理報告";
pub fn batch_report_operation(operation: &str) -> String {
    format!("要做的事：{operation}")
}
pub fn batch_report_folder(folder: &str) -> String {
    format!("資料夾：{folder}")
}
/// What a batch did, as its report says (never with its passwords).
pub fn batch_operation_name(operation: &BatchOperation) -> &'static str {
    match operation {
        BatchOperation::RemoveMetadata => "清除中繼資料",
        BatchOperation::Encrypt { .. } => "加密",
    }
}
/// How many files were done, failed and skipped: the parts that are not none.
pub fn batch_summary(done: u32, failed: u32, skipped: u32) -> String {
    let mut parts = vec![format!("完成 {done} 個")];
    if failed > 0 {
        parts.push(format!("失敗 {failed} 個"));
    }
    if skipped > 0 {
        parts.push(format!("略過 {skipped} 個"));
    }
    format!("{}。", parts.join("，"))
}
pub const BATCH_NOT_DONE: &str = "沒有處理";
/// How one file of a batch went, as the report says (the batch dialog says the same).
pub fn batch_outcome(outcome: &BatchOutcome) -> String {
    match outcome {
        BatchOutcome::Done { output } => format!("完成：{output}"),
        BatchOutcome::Failed { error } => format!("失敗：{}", batch_reason(*error)),
        BatchOutcome::Skipped => "略過（已停止）".to_owned(),
    }
}
fn batch_reason(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Encrypted => "有密碼的檔案，批次處理不詢問密碼",
        ErrorCode::InvalidArgument => "這個檔案不能這樣處理，例如有數位簽章的文件不能加密",
        ErrorCode::NotPdf => "不是 PDF 檔案",
        ErrorCode::Corrupted => "檔案已損毀",
        ErrorCode::UnsupportedEncryption => "使用本程式不支援的加密方式",
        ErrorCode::Unreadable => "無法讀取這個檔案",
        ErrorCode::TooLarge => "檔案太大",
        ErrorCode::LimitExceeded => "內容超過可處理的上限",
        ErrorCode::WorkerCrashed | ErrorCode::WorkerTimeout | ErrorCode::ProtocolViolation => {
            "PDF 引擎處理這個檔案時出錯"
        }
        ErrorCode::ReadOnly => "資料夾是唯讀的，無法寫入",
        ErrorCode::DiskFull => "磁碟空間不足",
        ErrorCode::FileInUse | ErrorCode::Unwritable => "無法寫入副本",
        _ => "發生未預期的錯誤",
    }
}
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
