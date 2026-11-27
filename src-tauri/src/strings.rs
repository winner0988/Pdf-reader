//! User-visible text shown by the main process itself and text written into files (native
//! dialogs, batch reports, annotation summaries, compare exports, visible signatures, page
//! number parts). Keep Traditional Chinese in sync with `docs/ux/screen-map.md`; the WebView's
//! text is in `src/i18n/`.

use ipc_contract::types::{AnnotationSummaryKind, BatchOperation, BatchOutcome, ErrorCode};
use ipc_contract::worker::{PageNumberParts, SigningTime};

/// Complete per-language strings used by the main process and written to output files.
///
/// Both `ZH_TW` and `EN` instantiate this struct so any missing field fails compilation.
pub struct Strings {
    pub page_number_parts: PageNumberParts,
    pub open_dialog_title: &'static str,
    pub pdf_filter_name: &'static str,
    pub pages_source_dialog_title: &'static str,
    pub save_as_dialog_title: &'static str,
    pub save_attachment_dialog_title: &'static str,
    pub attachment_type_filter_name: &'static str,
    pub all_files_filter_name: &'static str,

    pub privacy_export_dialog_title: &'static str,
    privacy_export_file_suffix: &'static str,
    pub privacy_export_same_file_title: &'static str,
    pub privacy_export_same_file_message: &'static str,

    pub sanitized_copy_dialog_title: &'static str,
    sanitized_copy_file_suffix: &'static str,
    pub sanitized_copy_same_file_title: &'static str,
    pub sanitized_copy_same_file_message: &'static str,

    pub encrypted_copy_dialog_title: &'static str,
    encrypted_copy_file_suffix: &'static str,
    pub encrypted_copy_same_file_title: &'static str,
    pub encrypted_copy_same_file_message: &'static str,

    pub decrypted_copy_dialog_title: &'static str,
    decrypted_copy_file_suffix: &'static str,
    pub decrypted_copy_same_file_title: &'static str,
    pub decrypted_copy_same_file_message: &'static str,

    pub compressed_copy_dialog_title: &'static str,
    compressed_copy_file_suffix: &'static str,
    pub compressed_copy_same_file_title: &'static str,
    pub compressed_copy_same_file_message: &'static str,

    exif_cleaned_file_suffix: &'static str,
    watermark_file_suffix: &'static str,

    pub page_marks_dialog_title: &'static str,
    page_marks_file_suffix: &'static str,
    pub page_marks_same_file_title: &'static str,
    pub page_marks_same_file_message: &'static str,

    pub searchable_copy_dialog_title: &'static str,
    searchable_copy_file_suffix: &'static str,
    pub searchable_copy_same_file_title: &'static str,
    pub searchable_copy_same_file_message: &'static str,

    pub certificate_dialog_title: &'static str,
    pub certificate_filter_name: &'static str,
    pub signed_copy_dialog_title: &'static str,
    signed_copy_file_suffix: &'static str,
    pub signed_copy_same_file_title: &'static str,
    pub signed_copy_same_file_message: &'static str,

    signature_signed_by_prefix: &'static str,
    signature_date_prefix: &'static str,
    signature_utc_open: &'static str,
    signature_utc_close: &'static str,
    signature_reason_prefix: &'static str,

    pub create_sources_dialog_title: &'static str,
    pub create_all_filter_name: &'static str,
    pub create_images_filter_name: &'static str,
    pub create_html_filter_name: &'static str,
    pub create_text_filter_name: &'static str,
    pub create_books_filter_name: &'static str,
    pub create_svg_filter_name: &'static str,
    pub create_office_filter_name: &'static str,
    pub create_pdf_dialog_title: &'static str,
    pub create_pdf_same_file_title: &'static str,
    pub create_pdf_same_file_message: &'static str,

    pub explorer_create_too_many_title: &'static str,
    explorer_create_too_many_prefix: &'static str,
    explorer_create_too_many_suffix: &'static str,
    pub explorer_create_skipped_title: &'static str,
    explorer_create_skipped_files_one: &'static str,
    explorer_create_skipped_files_other_suffix: &'static str,
    explorer_create_skipped_prefix: &'static str,
    explorer_create_skipped_middle: &'static str,
    explorer_create_skipped_images_one: &'static str,
    explorer_create_skipped_images_other_suffix: &'static str,
    explorer_create_skipped_suffix: &'static str,
    pub explorer_create_all_skipped_title: &'static str,
    explorer_create_all_skipped_prefix: &'static str,
    explorer_create_all_skipped_suffix: &'static str,

    pub language_dialog_title: &'static str,
    pub language_filter_name: &'static str,
    pub font_dialog_title: &'static str,
    pub font_filter_name: &'static str,

    pub export_text_dialog_title: &'static str,
    pub export_annotations_dialog_title: &'static str,
    pub markdown_filter_name: &'static str,
    annotation_summary_file_suffix: &'static str,
    pub annotation_summary_colon: &'static str,
    pub annotation_summary_pipe: &'static str,
    pub annotation_summary_truncated: &'static str,

    pub export_xfdf_dialog_title: &'static str,
    pub xfdf_filter_name: &'static str,
    xfdf_file_suffix: &'static str,

    kind_highlight: &'static str,
    kind_underline: &'static str,
    kind_strike_out: &'static str,
    kind_squiggly: &'static str,
    kind_note: &'static str,
    kind_text_box: &'static str,
    kind_ink: &'static str,
    kind_stamp: &'static str,
    kind_rectangle: &'static str,
    kind_ellipse: &'static str,
    kind_line: &'static str,
    kind_arrow: &'static str,

    pub export_compare_dialog_title: &'static str,
    compare_joiner: &'static str,
    compare_file_suffix: &'static str,
    compare_title_suffix: &'static str,
    compare_name_open: &'static str,
    compare_name_mid: &'static str,
    compare_name_close: &'static str,
    pub compare_label_deleted: &'static str,
    pub compare_label_inserted: &'static str,
    pub compare_colon: &'static str,
    pub compare_identical: &'static str,
    pub compare_empty: &'static str,
    compare_no_text_open: &'static str,
    compare_no_text_close: &'static str,
    pub compare_truncated: &'static str,
    pub compare_page_text_truncated: &'static str,
    pub compare_recognised_note: &'static str,

    pub export_docx_dialog_title: &'static str,
    pub docx_filter_name: &'static str,
    pub text_filter_name: &'static str,
    pub export_images_dialog_title: &'static str,
    pub stamp_image_dialog_title: &'static str,
    pub watermark_image_dialog_title: &'static str,
    pub image_filter_name: &'static str,

    pub split_dialog_title: &'static str,
    pub split_same_file_title: &'static str,
    pub split_same_file_message: &'static str,
    split_selected_suffix: &'static str,
    pub split_folder_dialog_title: &'static str,

    pub batch_files_dialog_title: &'static str,
    pub batch_folder_dialog_title: &'static str,
    batch_tray_tooltip_open: &'static str,
    batch_tray_tooltip_mid: &'static str,
    batch_tray_tooltip_close: &'static str,
    batch_summary_done_prefix: &'static str,
    batch_summary_done_suffix: &'static str,
    batch_summary_failed_prefix: &'static str,
    batch_summary_failed_suffix: &'static str,
    batch_summary_skipped_prefix: &'static str,
    batch_summary_skipped_suffix: &'static str,
    batch_summary_separator: &'static str,
    batch_summary_end: &'static str,
    pub batch_tray_show: &'static str,
    pub batch_tray_stop: &'static str,
    pub batch_done_title: &'static str,
    batch_done_report_prefix: &'static str,
    batch_done_report_failed: &'static str,
    pub batch_report_file_name: &'static str,
    pub batch_report_title: &'static str,
    batch_report_operation_prefix: &'static str,
    batch_report_folder_prefix: &'static str,
    batch_op_remove_metadata: &'static str,
    batch_op_remove_exif: &'static str,
    batch_op_encrypt: &'static str,
    batch_op_watermark: &'static str,
    batch_op_compress_images: &'static str,
    batch_op_sanitize: &'static str,
    batch_op_page_marks: &'static str,
    bates_range_separator: &'static str,
    pub batch_not_done: &'static str,
    batch_outcome_done_prefix: &'static str,
    batch_outcome_bates_open: &'static str,
    batch_outcome_bates_close: &'static str,
    batch_outcome_failed_prefix: &'static str,
    batch_outcome_skipped: &'static str,
    reason_encrypted: &'static str,
    reason_invalid_argument: &'static str,
    reason_signed: &'static str,
    reason_bates_overflow: &'static str,
    reason_no_page_to_mark: &'static str,
    reason_not_pdf: &'static str,
    reason_corrupted: &'static str,
    reason_unsupported_encryption: &'static str,
    reason_unreadable: &'static str,
    reason_too_large: &'static str,
    reason_limit_exceeded: &'static str,
    reason_worker_error: &'static str,
    reason_read_only: &'static str,
    reason_disk_full: &'static str,
    reason_unwritable: &'static str,
    reason_internal: &'static str,

    pub overwrite_title: &'static str,
    overwrite_one: &'static str,
    overwrite_many_prefix: &'static str,
    overwrite_many_suffix: &'static str,
    pub no_text_layer_page: &'static str,
    pub webview2_missing_title: &'static str,
    pub webview2_missing_message: &'static str,
}

impl Strings {
    /// The window title: the open document's file name, so windows and taskbar buttons can be
    /// told apart (REL-03), then the app's name. A dot marks unsaved changes (B2-02), as on the
    /// tab.
    pub fn window_title(&self, file_name: Option<&str>, unsaved: bool) -> String {
        match (file_name, unsaved) {
            (Some(name), true) => format!("• {name} — PDF Reader"),
            (Some(name), false) => format!("{name} — PDF Reader"),
            (None, _) => "PDF Reader".to_owned(),
        }
    }

    pub fn privacy_export_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.privacy_export_file_suffix)
    }

    pub fn sanitized_copy_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.sanitized_copy_file_suffix)
    }

    pub fn encrypted_copy_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.encrypted_copy_file_suffix)
    }

    pub fn decrypted_copy_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.decrypted_copy_file_suffix)
    }

    pub fn compressed_copy_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.compressed_copy_file_suffix)
    }

    pub fn exif_cleaned_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.exif_cleaned_file_suffix)
    }

    pub fn watermark_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.watermark_file_suffix)
    }

    pub fn page_marks_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.page_marks_file_suffix)
    }

    pub fn searchable_copy_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.searchable_copy_file_suffix)
    }

    pub fn signed_copy_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.signed_copy_file_suffix)
    }

    /// The lines in the box of a visible signature (B3-06): who signs, when, and why if given.
    pub fn signature_lines(
        &self,
        signer: &str,
        time: &SigningTime,
        reason: Option<&str>,
    ) -> Vec<String> {
        let offset = time.utc_offset_minutes;
        let mut lines = vec![
            format!("{}{signer}", self.signature_signed_by_prefix),
            format!(
                "{}{:04}-{:02}-{:02} {:02}:{:02}{}{}{:02}:{:02}{}",
                self.signature_date_prefix,
                time.year,
                time.month,
                time.day,
                time.hour,
                time.minute,
                self.signature_utc_open,
                if offset < 0 { '-' } else { '+' },
                offset.unsigned_abs() / 60,
                offset.unsigned_abs() % 60,
                self.signature_utc_close,
            ),
        ];
        if let Some(reason) = reason {
            lines.push(format!("{}{reason}", self.signature_reason_prefix));
        }
        lines
    }

    pub fn created_pdf_file_name(&self, stem: &str) -> String {
        format!("{stem}.pdf")
    }

    pub fn explorer_create_too_many_message(&self, limit: usize) -> String {
        format!(
            "{}{limit}{}",
            self.explorer_create_too_many_prefix, self.explorer_create_too_many_suffix
        )
    }

    pub fn explorer_create_skipped_message(&self, skipped: usize, valid: usize) -> String {
        let skipped_phrase = counted_phrase(
            skipped,
            self.explorer_create_skipped_files_one,
            self.explorer_create_skipped_files_other_suffix,
        );
        let valid_phrase = counted_phrase(
            valid,
            self.explorer_create_skipped_images_one,
            self.explorer_create_skipped_images_other_suffix,
        );
        format!(
            "{}{skipped_phrase}{}{valid_phrase}{}",
            self.explorer_create_skipped_prefix,
            self.explorer_create_skipped_middle,
            self.explorer_create_skipped_suffix,
        )
    }

    pub fn explorer_create_all_skipped_message(&self, skipped: usize) -> String {
        let skipped_phrase = counted_phrase(
            skipped,
            self.explorer_create_skipped_files_one,
            self.explorer_create_skipped_files_other_suffix,
        );
        format!(
            "{}{skipped_phrase}{}",
            self.explorer_create_all_skipped_prefix, self.explorer_create_all_skipped_suffix,
        )
    }

    pub fn annotation_summary_file_name(&self, stem: &str, markdown: bool) -> String {
        let extension = if markdown { "md" } else { "txt" };
        format!("{stem}{}.{extension}", self.annotation_summary_file_suffix)
    }

    pub fn xfdf_file_name(&self, stem: &str) -> String {
        format!("{stem}{}", self.xfdf_file_suffix)
    }

    pub fn annotation_kind_name(&self, kind: AnnotationSummaryKind) -> &'static str {
        match kind {
            AnnotationSummaryKind::Highlight => self.kind_highlight,
            AnnotationSummaryKind::Underline => self.kind_underline,
            AnnotationSummaryKind::StrikeOut => self.kind_strike_out,
            AnnotationSummaryKind::Squiggly => self.kind_squiggly,
            AnnotationSummaryKind::Note => self.kind_note,
            AnnotationSummaryKind::TextBox => self.kind_text_box,
            AnnotationSummaryKind::Ink => self.kind_ink,
            AnnotationSummaryKind::Stamp => self.kind_stamp,
            AnnotationSummaryKind::Rectangle => self.kind_rectangle,
            AnnotationSummaryKind::Ellipse => self.kind_ellipse,
            AnnotationSummaryKind::Line => self.kind_line,
            AnnotationSummaryKind::Arrow => self.kind_arrow,
        }
    }

    /// The page an annotation is on, counted from 1.
    pub fn annotation_summary_page(&self, page: u32) -> String {
        format!(
            "{}{page}{}",
            self.page_number_parts.page_before, self.page_number_parts.page_after
        )
    }

    /// Default file name for text comparison export (B3-24, E9).
    pub fn compare_file_name(&self, stem_a: &str, stem_b: &str, markdown: bool) -> String {
        let extension = if markdown { "md" } else { "txt" };
        format!(
            "{stem_a}{}{stem_b}{}.{extension}",
            self.compare_joiner, self.compare_file_suffix
        )
    }

    /// The title of the text comparison file (B3-24, E9).
    pub fn compare_title(&self, name_a: &str, name_b: &str) -> String {
        format!(
            "{name_a}{}{name_b}{}",
            self.compare_joiner, self.compare_title_suffix
        )
    }

    /// The page header for an item in the text comparison file (B3-24, E9).
    pub fn compare_item_header(
        &self,
        page_a: u32,
        name_a: &str,
        page_b: u32,
        name_b: &str,
    ) -> String {
        let p_a = self.annotation_summary_page(page_a);
        let p_b = self.annotation_summary_page(page_b);
        format!(
            "{p_a}{}{name_a}{}{p_b}{}{name_b}{}",
            self.compare_name_open,
            self.compare_name_mid,
            self.compare_name_open,
            self.compare_name_close,
        )
    }

    pub fn compare_no_text_one_side(&self, name: &str) -> String {
        format!(
            "{}{name}{}",
            self.compare_no_text_open, self.compare_no_text_close
        )
    }

    pub fn batch_tray_tooltip(&self, done: u32, total: u32) -> String {
        format!(
            "{}{done}{}{total}{}",
            self.batch_tray_tooltip_open,
            self.batch_tray_tooltip_mid,
            self.batch_tray_tooltip_close
        )
    }

    pub fn batch_done_message(&self, summary: &str, report: Option<&str>) -> String {
        match report {
            Some(path) => format!("{summary}\n\n{}\n{path}", self.batch_done_report_prefix),
            None => format!("{summary}\n\n{}", self.batch_done_report_failed),
        }
    }

    pub fn batch_report_operation(&self, operation: &str) -> String {
        format!("{}{operation}", self.batch_report_operation_prefix)
    }

    pub fn batch_report_folder(&self, folder: &str) -> String {
        format!("{}{folder}", self.batch_report_folder_prefix)
    }

    /// What a batch did, as its report says (never with its passwords).
    pub fn batch_operation_name(&self, operation: &BatchOperation) -> &'static str {
        match operation {
            BatchOperation::RemoveMetadata => self.batch_op_remove_metadata,
            BatchOperation::RemoveExif => self.batch_op_remove_exif,
            BatchOperation::Encrypt { .. } => self.batch_op_encrypt,
            BatchOperation::Watermark { .. } => self.batch_op_watermark,
            BatchOperation::CompressImages { .. } => self.batch_op_compress_images,
            BatchOperation::Sanitize { .. } => self.batch_op_sanitize,
            BatchOperation::PageMarks { .. } => self.batch_op_page_marks,
        }
    }

    /// The number range of a file in a Bates batch (ADR 0024): `000001～000010` or `000001–000010`,
    /// or `000001` for one page.
    pub fn bates_range(&self, first: &str, last: &str) -> String {
        if first == last {
            first.to_owned()
        } else {
            format!("{first}{}{last}", self.bates_range_separator)
        }
    }

    /// How many files were done, failed and skipped: the parts that are not zero (except done).
    pub fn batch_summary(&self, done: u32, failed: u32, skipped: u32) -> String {
        let mut parts = vec![format!(
            "{}{done}{}",
            self.batch_summary_done_prefix, self.batch_summary_done_suffix
        )];
        if failed > 0 {
            parts.push(format!(
                "{}{failed}{}",
                self.batch_summary_failed_prefix, self.batch_summary_failed_suffix
            ));
        }
        if skipped > 0 {
            parts.push(format!(
                "{}{skipped}{}",
                self.batch_summary_skipped_prefix, self.batch_summary_skipped_suffix
            ));
        }
        format!(
            "{}{}",
            parts.join(self.batch_summary_separator),
            self.batch_summary_end
        )
    }

    /// How one file of a batch went, as the report says.
    pub fn batch_outcome(&self, outcome: &BatchOutcome) -> String {
        match outcome {
            BatchOutcome::Done { output, bates } => match bates {
                Some(range) => format!(
                    "{}{output}{}{}{}",
                    self.batch_outcome_done_prefix,
                    self.batch_outcome_bates_open,
                    self.bates_range(&range.first, &range.last),
                    self.batch_outcome_bates_close,
                ),
                None => format!("{}{output}", self.batch_outcome_done_prefix),
            },
            BatchOutcome::Failed { error } => {
                format!(
                    "{}{}",
                    self.batch_outcome_failed_prefix,
                    self.batch_reason(*error)
                )
            }
            BatchOutcome::Skipped => self.batch_outcome_skipped.to_owned(),
        }
    }

    fn batch_reason(&self, code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::Encrypted => self.reason_encrypted,
            ErrorCode::InvalidArgument => self.reason_invalid_argument,
            ErrorCode::Signed => self.reason_signed,
            ErrorCode::BatesOverflow => self.reason_bates_overflow,
            ErrorCode::NoPageToMark => self.reason_no_page_to_mark,
            ErrorCode::NotPdf => self.reason_not_pdf,
            ErrorCode::Corrupted => self.reason_corrupted,
            ErrorCode::UnsupportedEncryption => self.reason_unsupported_encryption,
            ErrorCode::Unreadable => self.reason_unreadable,
            ErrorCode::TooLarge => self.reason_too_large,
            ErrorCode::LimitExceeded => self.reason_limit_exceeded,
            ErrorCode::WorkerCrashed | ErrorCode::WorkerTimeout | ErrorCode::ProtocolViolation => {
                self.reason_worker_error
            }
            ErrorCode::ReadOnly => self.reason_read_only,
            ErrorCode::DiskFull => self.reason_disk_full,
            ErrorCode::FileInUse | ErrorCode::Unwritable => self.reason_unwritable,
            _ => self.reason_internal,
        }
    }

    /// The name suggested for a file of some pages of a document (B2-06): the pages' numbers when
    /// they follow one another, else only that they were chosen.
    pub fn split_file_name(&self, stem: &str, pages: &[u32]) -> String {
        match (pages.first(), pages.last()) {
            (Some(first), Some(last)) if follow_one_another(pages) => {
                let (first, last) = (first.saturating_add(1), last.saturating_add(1));
                if first == last {
                    format!("{stem}-p{first}.pdf")
                } else {
                    format!("{stem}-p{first}-{last}.pdf")
                }
            }
            _ => format!("{stem}{}", self.split_selected_suffix),
        }
    }

    /// The name of part number `part` (0-based) of `parts` that a document is split into (B2-06):
    /// the pages' numbers when they follow one another, else the number of the part.
    pub fn split_part_name(&self, stem: &str, pages: &[u32], part: usize, parts: usize) -> String {
        if follow_one_another(pages) {
            self.split_file_name(stem, pages)
        } else {
            let width = parts.to_string().len();
            format!("{stem}-{:0width$}.pdf", part + 1)
        }
    }

    /// Asked before exported page images replace files already in the chosen folder.
    pub fn overwrite_message(&self, count: usize) -> String {
        if count == 1 {
            self.overwrite_one.to_owned()
        } else {
            format!(
                "{}{count}{}",
                self.overwrite_many_prefix, self.overwrite_many_suffix
            )
        }
    }
}

fn counted_phrase(count: usize, one: &str, other_suffix: &str) -> String {
    if count == 1 {
        one.to_owned()
    } else {
        format!("{count}{other_suffix}")
    }
}

fn follow_one_another(pages: &[u32]) -> bool {
    pages
        .windows(2)
        .all(|pair| pair[0].checked_add(1) == Some(pair[1]))
}

/// Returns the string table matching an effective language code (`"en"` or `"zh-TW"`).
pub fn for_language(language: &str) -> &'static Strings {
    match language {
        "en" => &EN,
        _ => &ZH_TW,
    }
}

pub static ZH_TW: Strings = Strings {
    page_number_parts: PageNumberParts {
        page_before: "第 ",
        page_after: " 頁",
        total_before: "第 ",
        total_middle: " 頁／共 ",
        total_after: " 頁",
    },
    open_dialog_title: "開啟 PDF 檔案",
    pdf_filter_name: "PDF 檔案",
    pages_source_dialog_title: "選擇要插入頁面的 PDF 檔案",
    save_as_dialog_title: "另存新檔",
    save_attachment_dialog_title: "另存附件",
    attachment_type_filter_name: "附件的類型",
    all_files_filter_name: "所有檔案",

    privacy_export_dialog_title: "隱私匯出：選擇副本的位置",
    privacy_export_file_suffix: "（隱私匯出）.pdf",
    privacy_export_same_file_title: "請選擇其他檔案",
    privacy_export_same_file_message: "隱私匯出會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    sanitized_copy_dialog_title: "分享前淨化：選擇副本的位置",
    sanitized_copy_file_suffix: "（淨化副本）.pdf",
    sanitized_copy_same_file_title: "請選擇其他檔案",
    sanitized_copy_same_file_message: "分享前淨化會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    encrypted_copy_dialog_title: "加密並另存新檔：選擇副本的位置",
    encrypted_copy_file_suffix: "（已加密）.pdf",
    encrypted_copy_same_file_title: "請選擇其他檔案",
    encrypted_copy_same_file_message: "加密會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    decrypted_copy_dialog_title: "另存沒有密碼的副本：選擇副本的位置",
    decrypted_copy_file_suffix: "（無密碼）.pdf",
    decrypted_copy_same_file_title: "請選擇其他檔案",
    decrypted_copy_same_file_message: "另存沒有密碼的副本會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    compressed_copy_dialog_title: "壓縮圖片並另存新檔：選擇副本的位置",
    compressed_copy_file_suffix: "（已壓縮）.pdf",
    compressed_copy_same_file_title: "請選擇其他檔案",
    compressed_copy_same_file_message: "壓縮會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    exif_cleaned_file_suffix: "（已清除 EXIF）.pdf",
    watermark_file_suffix: "（浮水印）.pdf",

    page_marks_dialog_title: "頁碼與 Bates 編號：選擇副本的位置",
    page_marks_file_suffix: "（頁碼與 Bates）.pdf",
    page_marks_same_file_title: "請選擇其他檔案",
    page_marks_same_file_message: "頁碼與 Bates 編號會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    searchable_copy_dialog_title: "另存為可搜尋的 PDF：選擇副本的位置",
    searchable_copy_file_suffix: "（可搜尋）.pdf",
    searchable_copy_same_file_title: "請選擇其他檔案",
    searchable_copy_same_file_message: "另存為可搜尋的 PDF 會產生一份副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    certificate_dialog_title: "選擇簽署用的憑證檔",
    certificate_filter_name: "憑證檔（PKCS #12）",
    signed_copy_dialog_title: "簽署並另存新檔：選擇已簽署副本的位置",
    signed_copy_file_suffix: "（已簽署）.pdf",
    signed_copy_same_file_title: "請選擇其他檔案",
    signed_copy_same_file_message: "簽署會產生一份已簽署的副本，不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    signature_signed_by_prefix: "數位簽署：",
    signature_date_prefix: "日期：",
    signature_utc_open: "（UTC",
    signature_utc_close: "）",
    signature_reason_prefix: "原因：",

    create_sources_dialog_title: "選擇要建立 PDF 的檔案",
    create_all_filter_name: "所有可以建立 PDF 的檔案",
    create_images_filter_name: "圖片（PNG、JPEG、GIF、BMP、TIFF）",
    create_html_filter_name: "網頁（HTML）",
    create_text_filter_name: "純文字（TXT）",
    create_books_filter_name: "電子書與固定版面（EPUB、XPS）",
    create_svg_filter_name: "SVG 向量圖",
    create_office_filter_name: "Office 文件（DOCX、XLSX、PPTX；實驗性）",
    create_pdf_dialog_title: "建立 PDF：選擇 PDF 的位置",
    create_pdf_same_file_title: "請選擇其他檔案",
    create_pdf_same_file_message: "建立 PDF 不會改動原本的檔案。請選擇原檔以外的位置或檔名。",

    explorer_create_too_many_title: "圖片數量超過上限",
    explorer_create_too_many_prefix: "一次最多只能使用 ",
    explorer_create_too_many_suffix: " 張圖片建立 PDF，請減少選取的圖片後再試一次。",
    explorer_create_skipped_title: "部分檔案已略過",
    explorer_create_skipped_files_one: "1 個檔案",
    explorer_create_skipped_files_other_suffix: " 個檔案",
    explorer_create_skipped_prefix: "已略過 ",
    explorer_create_skipped_middle: "：只支援存在的一般圖片檔（PNG、JPEG、GIF、BMP、TIFF）。\n\n接下來會用其餘 ",
    explorer_create_skipped_images_one: "1 張圖片",
    explorer_create_skipped_images_other_suffix: " 張圖片",
    explorer_create_skipped_suffix: "建立 PDF。",
    explorer_create_all_skipped_title: "無法建立 PDF",
    explorer_create_all_skipped_prefix: "已略過 ",
    explorer_create_all_skipped_suffix: "，沒有可以用來建立 PDF 的圖片。\n\n只支援存在的一般圖片檔（PNG、JPEG、GIF、BMP、TIFF）。",

    language_dialog_title: "匯入 OCR 語言資料",
    language_filter_name: "Tesseract 語言資料（.traineddata）",
    font_dialog_title: "匯入字型檔",
    font_filter_name: "字型檔（.ttf、.ttc）",

    export_text_dialog_title: "匯出純文字",
    export_annotations_dialog_title: "匯出註解摘要",
    markdown_filter_name: "Markdown 文件（.md）",
    annotation_summary_file_suffix: "（註解摘要）",
    annotation_summary_colon: "：",
    annotation_summary_pipe: "｜",
    annotation_summary_truncated: "（註解太多，其餘沒有列出。）",

    export_xfdf_dialog_title: "匯出註解（XFDF）",
    xfdf_filter_name: "XFDF (*.xfdf)",
    xfdf_file_suffix: "（註解）.xfdf",

    kind_highlight: "螢光筆標示",
    kind_underline: "底線標示",
    kind_strike_out: "刪除線標示",
    kind_squiggly: "波浪線標示",
    kind_note: "附註",
    kind_text_box: "文字方塊",
    kind_ink: "手繪線條",
    kind_stamp: "印章",
    kind_rectangle: "矩形",
    kind_ellipse: "橢圓",
    kind_line: "直線",
    kind_arrow: "箭頭",

    export_compare_dialog_title: "匯出文字比較結果",
    compare_joiner: " 與 ",
    compare_file_suffix: "（文字比較）",
    compare_title_suffix: "（文字比較）",
    compare_name_open: "（",
    compare_name_mid: "）/ ",
    compare_name_close: "）",
    compare_label_deleted: "刪除",
    compare_label_inserted: "新增",
    compare_colon: "：",
    compare_identical: "兩份文件文字完全相同。",
    compare_empty: "兩份文件皆未包含可比較的文字。",
    compare_no_text_open: "（「",
    compare_no_text_close: "」未包含可比較的文字，建議先執行文字辨識。）",
    compare_truncated: "（差異過多，其餘未列出。）",
    compare_page_text_truncated: "（部分頁面的文字過長已被截斷。）",
    compare_recognised_note: "（部分文字由光學字元辨識取得，可能有誤。）",

    export_docx_dialog_title: "匯出 Word 文件",
    docx_filter_name: "Word 文件（.docx）",
    text_filter_name: "純文字檔",
    export_images_dialog_title: "選擇匯出頁面圖片的資料夾",
    stamp_image_dialog_title: "選擇印章要用的圖片",
    watermark_image_dialog_title: "選擇浮水印要用的圖片",
    image_filter_name: "圖片（PNG、JPEG）",

    split_dialog_title: "將選取的頁面另存為新檔",
    split_same_file_title: "請選擇其他檔案",
    split_same_file_message: "拆分出的頁面是一份新的檔案，不會取代原本的檔案。請選擇原檔以外的位置或檔名。",
    split_selected_suffix: "（選取的頁面）.pdf",
    split_folder_dialog_title: "選擇存放拆分後檔案的資料夾",

    batch_files_dialog_title: "選擇要批次處理的 PDF 檔案",
    batch_folder_dialog_title: "選擇存放批次處理結果的資料夾",
    batch_tray_tooltip_open: "PDF Reader：批次處理中（",
    batch_tray_tooltip_mid: "／",
    batch_tray_tooltip_close: "），完成後就會結束",
    batch_summary_done_prefix: "完成 ",
    batch_summary_done_suffix: " 個",
    batch_summary_failed_prefix: "失敗 ",
    batch_summary_failed_suffix: " 個",
    batch_summary_skipped_prefix: "略過 ",
    batch_summary_skipped_suffix: " 個",
    batch_summary_separator: "，",
    batch_summary_end: "。",
    batch_tray_show: "顯示視窗",
    batch_tray_stop: "停止批次處理",
    batch_done_title: "批次處理完成",
    batch_done_report_prefix: "每個檔案的結果寫在：",
    batch_done_report_failed: "報告無法寫進資料夾。",
    batch_report_file_name: "批次處理報告.txt",
    batch_report_title: "PDF Reader 批次處理報告",
    batch_report_operation_prefix: "要做的事：",
    batch_report_folder_prefix: "資料夾：",
    batch_op_remove_metadata: "清除中繼資料",
    batch_op_remove_exif: "清除圖片的 EXIF",
    batch_op_encrypt: "加密",
    batch_op_watermark: "浮水印",
    batch_op_compress_images: "壓縮圖片",
    batch_op_sanitize: "分享前淨化",
    batch_op_page_marks: "頁碼與 Bates 編號",
    bates_range_separator: "～",
    batch_not_done: "沒有處理",
    batch_outcome_done_prefix: "完成：",
    batch_outcome_bates_open: "（",
    batch_outcome_bates_close: "）",
    batch_outcome_failed_prefix: "失敗：",
    batch_outcome_skipped: "略過（已停止）",
    reason_encrypted: "有密碼的檔案，批次處理不詢問密碼",
    reason_invalid_argument: "這個檔案不能這樣處理，例如有數位簽章的文件不能加密或壓縮圖片（簽章會失效）",
    reason_signed: "有數位簽章的檔案，這樣處理會讓簽章失效",
    reason_bates_overflow: "這個檔案的 Bates 編號超出位數上限",
    reason_no_page_to_mark: "這個檔案沒有要加標記的頁面，或頁面範圍超出它的頁數",
    reason_not_pdf: "不是 PDF 檔案",
    reason_corrupted: "檔案已損毀",
    reason_unsupported_encryption: "使用本程式不支援的加密方式",
    reason_unreadable: "無法讀取這個檔案",
    reason_too_large: "檔案太大",
    reason_limit_exceeded: "內容超過可處理的上限",
    reason_worker_error: "PDF 引擎處理這個檔案時出錯",
    reason_read_only: "資料夾是唯讀的，無法寫入",
    reason_disk_full: "磁碟空間不足",
    reason_unwritable: "無法寫入副本",
    reason_internal: "發生未預期的錯誤",

    overwrite_title: "檔案已經存在",
    overwrite_one: "這個資料夾已經有 1 個同名的檔案。要覆寫嗎？",
    overwrite_many_prefix: "這個資料夾已經有 ",
    overwrite_many_suffix: " 個同名的檔案。要覆寫嗎？",
    no_text_layer_page: "（此頁沒有文字層）",
    webview2_missing_title: "無法開啟 PDF Reader",
    webview2_missing_message: "這台電腦缺少 Microsoft Edge WebView2 Runtime，PDF Reader 需要它才能顯示畫面。\n\n\
Windows 11 已內建 WebView2。如果它被移除了，請到 Microsoft 官方網站下載並安裝「WebView2 Runtime」，然後再開啟 PDF Reader：\n\
https://developer.microsoft.com/microsoft-edge/webview2/\n\n\
PDF Reader 不會自行下載任何東西。",
};

pub static EN: Strings = Strings {
    page_number_parts: PageNumberParts {
        page_before: "Page ",
        page_after: "",
        total_before: "Page ",
        total_middle: " of ",
        total_after: "",
    },
    open_dialog_title: "Open PDF File",
    pdf_filter_name: "PDF files",
    pages_source_dialog_title: "Select PDF File to Insert Pages From",
    save_as_dialog_title: "Save As",
    save_attachment_dialog_title: "Save Attachment As",
    attachment_type_filter_name: "Attachment type",
    all_files_filter_name: "All files",

    privacy_export_dialog_title: "Privacy Export: Choose Location for Copy",
    privacy_export_file_suffix: " (privacy export).pdf",
    privacy_export_same_file_title: "Please Choose Another File",
    privacy_export_same_file_message: "Privacy export creates a copy without modifying the original file. Please choose a different location or filename.",

    sanitized_copy_dialog_title: "Sanitize Copy: Choose Location for Copy",
    sanitized_copy_file_suffix: " (sanitized copy).pdf",
    sanitized_copy_same_file_title: "Please Choose Another File",
    sanitized_copy_same_file_message: "Sanitize copy creates a copy without modifying the original file. Please choose a different location or filename.",

    encrypted_copy_dialog_title: "Encrypt and Save As: Choose Location for Copy",
    encrypted_copy_file_suffix: " (encrypted).pdf",
    encrypted_copy_same_file_title: "Please Choose Another File",
    encrypted_copy_same_file_message: "Encryption creates a copy without modifying the original file. Please choose a different location or filename.",

    decrypted_copy_dialog_title: "Save Copy Without Passwords: Choose Location for Copy",
    decrypted_copy_file_suffix: " (no password).pdf",
    decrypted_copy_same_file_title: "Please Choose Another File",
    decrypted_copy_same_file_message: "Saving a copy without passwords creates a copy without modifying the original file. Please choose a different location or filename.",

    compressed_copy_dialog_title: "Compress Images and Save As: Choose Location for Copy",
    compressed_copy_file_suffix: " (compressed).pdf",
    compressed_copy_same_file_title: "Please Choose Another File",
    compressed_copy_same_file_message: "Compression creates a copy without modifying the original file. Please choose a different location or filename.",

    exif_cleaned_file_suffix: " (EXIF cleaned).pdf",
    watermark_file_suffix: " (watermark).pdf",

    page_marks_dialog_title: "Page Numbers and Bates Numbering: Choose Location for Copy",
    page_marks_file_suffix: " (page marks and Bates).pdf",
    page_marks_same_file_title: "Please Choose Another File",
    page_marks_same_file_message: "Page numbers and Bates numbering create a copy without modifying the original file. Please choose a different location or filename.",

    searchable_copy_dialog_title: "Save as Searchable PDF: Choose Location for Copy",
    searchable_copy_file_suffix: " (searchable).pdf",
    searchable_copy_same_file_title: "Please Choose Another File",
    searchable_copy_same_file_message: "Saving as searchable PDF creates a copy without modifying the original file. Please choose a different location or filename.",

    certificate_dialog_title: "Choose Signing Certificate File",
    certificate_filter_name: "Certificate files (PKCS #12)",
    signed_copy_dialog_title: "Sign and Save As: Choose Location for Signed Copy",
    signed_copy_file_suffix: " (signed).pdf",
    signed_copy_same_file_title: "Please Choose Another File",
    signed_copy_same_file_message: "Signing creates a signed copy without modifying the original file. Please choose a different location or filename.",

    signature_signed_by_prefix: "Digitally signed by: ",
    signature_date_prefix: "Date: ",
    signature_utc_open: " (UTC",
    signature_utc_close: ")",
    signature_reason_prefix: "Reason: ",

    create_sources_dialog_title: "Select Files to Create PDF",
    create_all_filter_name: "All supported files",
    create_images_filter_name: "Images (PNG, JPEG, GIF, BMP, TIFF)",
    create_html_filter_name: "Web pages (HTML)",
    create_text_filter_name: "Plain text (TXT)",
    create_books_filter_name: "E-books and fixed-layout (EPUB, XPS)",
    create_svg_filter_name: "SVG vector graphics",
    create_office_filter_name: "Office documents (DOCX, XLSX, PPTX; experimental)",
    create_pdf_dialog_title: "Create PDF: Choose Destination for PDF",
    create_pdf_same_file_title: "Please Choose Another File",
    create_pdf_same_file_message: "Creating a PDF does not modify the source files. Please choose a different location or filename.",

    explorer_create_too_many_title: "Image Limit Exceeded",
    explorer_create_too_many_prefix: "At most ",
    explorer_create_too_many_suffix: " images can be used to create a PDF at once. Please select fewer images and try again.",
    explorer_create_skipped_title: "Some Files Skipped",
    explorer_create_skipped_files_one: "1 file",
    explorer_create_skipped_files_other_suffix: " files",
    explorer_create_skipped_prefix: "Skipped ",
    explorer_create_skipped_middle: ": only existing regular image files (PNG, JPEG, GIF, BMP, TIFF) are supported.\n\nA PDF will be created with the remaining ",
    explorer_create_skipped_images_one: "1 image",
    explorer_create_skipped_images_other_suffix: " images",
    explorer_create_skipped_suffix: ".",
    explorer_create_all_skipped_title: "Cannot Create PDF",
    explorer_create_all_skipped_prefix: "Skipped ",
    explorer_create_all_skipped_suffix: "; no usable images found to create a PDF.\n\nOnly existing regular image files (PNG, JPEG, GIF, BMP, TIFF) are supported.",

    language_dialog_title: "Import OCR Language Data",
    language_filter_name: "Tesseract language data (.traineddata)",
    font_dialog_title: "Import Font File",
    font_filter_name: "Font files (.ttf, .ttc)",

    export_text_dialog_title: "Export Plain Text",
    export_annotations_dialog_title: "Export Annotation Summary",
    markdown_filter_name: "Markdown documents (.md)",
    annotation_summary_file_suffix: " (annotation summary)",
    annotation_summary_colon: ": ",
    annotation_summary_pipe: " | ",
    annotation_summary_truncated: "(Too many annotations; remainder omitted.)",

    export_xfdf_dialog_title: "Export Annotations (XFDF)",
    xfdf_filter_name: "XFDF (*.xfdf)",
    xfdf_file_suffix: " (annotations).xfdf",

    kind_highlight: "Highlight",
    kind_underline: "Underline",
    kind_strike_out: "Strikethrough",
    kind_squiggly: "Squiggly",
    kind_note: "Note",
    kind_text_box: "Text box",
    kind_ink: "Drawing",
    kind_stamp: "Stamp",
    kind_rectangle: "Rectangle",
    kind_ellipse: "Ellipse",
    kind_line: "Line",
    kind_arrow: "Arrow",

    export_compare_dialog_title: "Export Text Comparison",
    compare_joiner: " vs ",
    compare_file_suffix: " (text comparison)",
    compare_title_suffix: " (Text Comparison)",
    compare_name_open: " (",
    compare_name_mid: ") / ",
    compare_name_close: ")",
    compare_label_deleted: "Deleted",
    compare_label_inserted: "Added",
    compare_colon: ": ",
    compare_identical: "Text content in both documents is identical.",
    compare_empty: "Neither document contains comparable text.",
    compare_no_text_open: "(\"",
    compare_no_text_close: "\" contains no comparable text; text recognition (OCR) is recommended first.)",
    compare_truncated: "(Too many differences; remainder omitted.)",
    compare_page_text_truncated: "(Text on some pages was truncated due to length.)",
    compare_recognised_note: "(Some text was recognized by OCR and may contain inaccuracies.)",

    export_docx_dialog_title: "Export Word Document",
    docx_filter_name: "Word documents (.docx)",
    text_filter_name: "Plain text files",
    export_images_dialog_title: "Choose Folder for Exported Page Images",
    stamp_image_dialog_title: "Choose Image for Stamp",
    watermark_image_dialog_title: "Choose Image for Watermark",
    image_filter_name: "Images (PNG, JPEG)",

    split_dialog_title: "Save Selected Pages as New File",
    split_same_file_title: "Please Choose Another File",
    split_same_file_message: "Split pages are saved to a new file and will not replace the original file. Please choose a different location or filename.",
    split_selected_suffix: " (selected pages).pdf",
    split_folder_dialog_title: "Choose Folder for Split Files",

    batch_files_dialog_title: "Choose PDF Files for Batch Processing",
    batch_folder_dialog_title: "Choose Folder for Batch Output",
    batch_tray_tooltip_open: "PDF Reader: Batch processing (",
    batch_tray_tooltip_mid: " / ",
    batch_tray_tooltip_close: "); will exit when done",
    batch_summary_done_prefix: "Completed ",
    batch_summary_done_suffix: "",
    batch_summary_failed_prefix: "Failed ",
    batch_summary_failed_suffix: "",
    batch_summary_skipped_prefix: "Skipped ",
    batch_summary_skipped_suffix: "",
    batch_summary_separator: ", ",
    batch_summary_end: ".",
    batch_tray_show: "Show Window",
    batch_tray_stop: "Stop Batch Processing",
    batch_done_title: "Batch Processing Complete",
    batch_done_report_prefix: "Per-file results saved to:",
    batch_done_report_failed: "Could not write report to folder.",
    batch_report_file_name: "Batch Processing Report.txt",
    batch_report_title: "PDF Reader Batch Processing Report",
    batch_report_operation_prefix: "Operation: ",
    batch_report_folder_prefix: "Folder: ",
    batch_op_remove_metadata: "Clear metadata",
    batch_op_remove_exif: "Clear image EXIF",
    batch_op_encrypt: "Encrypt",
    batch_op_watermark: "Watermark",
    batch_op_compress_images: "Compress images",
    batch_op_sanitize: "Sanitize before sharing",
    batch_op_page_marks: "Page numbers and Bates numbering",
    bates_range_separator: "–",
    batch_not_done: "Not processed",
    batch_outcome_done_prefix: "Done: ",
    batch_outcome_bates_open: " (",
    batch_outcome_bates_close: ")",
    batch_outcome_failed_prefix: "Failed: ",
    batch_outcome_skipped: "Skipped (stopped)",
    reason_encrypted: "Password-protected file; batch processing does not prompt for passwords",
    reason_invalid_argument: "File cannot be processed this way (e.g. signed documents cannot be encrypted or compressed, as signatures would fail)",
    reason_signed: "Digitally signed file; processing would invalidate signature",
    reason_bates_overflow: "Bates numbering for this file exceeds digit capacity",
    reason_no_page_to_mark: "This file has no pages to mark, or range exceeds page count",
    reason_not_pdf: "Not a PDF file",
    reason_corrupted: "File corrupted",
    reason_unsupported_encryption: "Encryption method not supported by this program",
    reason_unreadable: "Could not read this file",
    reason_too_large: "File too large",
    reason_limit_exceeded: "Content exceeds processing limits",
    reason_worker_error: "PDF engine encountered an error while processing this file",
    reason_read_only: "Folder is read-only and cannot be written to",
    reason_disk_full: "Insufficient disk space",
    reason_unwritable: "Could not write copy",
    reason_internal: "An unexpected error occurred",

    overwrite_title: "File Already Exists",
    overwrite_one: "There is already 1 file with the same name in this folder. Overwrite it?",
    overwrite_many_prefix: "There are already ",
    overwrite_many_suffix: " files with the same name in this folder. Overwrite them?",
    no_text_layer_page: "(This page has no text layer)",
    webview2_missing_title: "Cannot Start PDF Reader",
    webview2_missing_message: "This computer is missing Microsoft Edge WebView2 Runtime, which PDF Reader needs to display its window.\n\n\
Windows 11 includes WebView2. If it was removed, please download and install \"WebView2 Runtime\" from the official Microsoft website, then restart PDF Reader:\n\
https://developer.microsoft.com/microsoft-edge/webview2/\n\n\
PDF Reader never downloads anything on its own.",
};

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{BatchOperation, BatchOutcome, EN, ErrorCode, Strings, ZH_TW, for_language};

    fn is_hanzi_or_cjk_punctuation(c: char) -> bool {
        matches!(
            c as u32,
            0x3000..=0x303F
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xF900..=0xFAFF
                | 0xFF00..=0xFFEF
                | 0x20000..=0x2CEAF
        )
    }

    fn is_hanzi(c: char) -> bool {
        matches!(
            c as u32,
            0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2CEAF
        )
    }

    fn check_en_text(name: &str, value: &str, allow_empty: bool) {
        if !allow_empty {
            assert!(!value.is_empty(), "EN.{name} must not be empty");
        }
        assert!(
            !value.chars().any(is_hanzi_or_cjk_punctuation),
            "EN.{name} contains CJK or fullwidth characters: {value:?}"
        );
    }

    fn check_struct_strings(s: &Strings) {
        // Every `&'static str` field in Strings checked explicitly.
        let required_fields = [
            (
                "page_number_parts.page_before",
                s.page_number_parts.page_before,
            ),
            (
                "page_number_parts.total_before",
                s.page_number_parts.total_before,
            ),
            (
                "page_number_parts.total_middle",
                s.page_number_parts.total_middle,
            ),
            ("open_dialog_title", s.open_dialog_title),
            ("pdf_filter_name", s.pdf_filter_name),
            ("pages_source_dialog_title", s.pages_source_dialog_title),
            ("save_as_dialog_title", s.save_as_dialog_title),
            (
                "save_attachment_dialog_title",
                s.save_attachment_dialog_title,
            ),
            ("attachment_type_filter_name", s.attachment_type_filter_name),
            ("all_files_filter_name", s.all_files_filter_name),
            ("privacy_export_dialog_title", s.privacy_export_dialog_title),
            ("privacy_export_file_suffix", s.privacy_export_file_suffix),
            (
                "privacy_export_same_file_title",
                s.privacy_export_same_file_title,
            ),
            (
                "privacy_export_same_file_message",
                s.privacy_export_same_file_message,
            ),
            ("sanitized_copy_dialog_title", s.sanitized_copy_dialog_title),
            ("sanitized_copy_file_suffix", s.sanitized_copy_file_suffix),
            (
                "sanitized_copy_same_file_title",
                s.sanitized_copy_same_file_title,
            ),
            (
                "sanitized_copy_same_file_message",
                s.sanitized_copy_same_file_message,
            ),
            ("encrypted_copy_dialog_title", s.encrypted_copy_dialog_title),
            ("encrypted_copy_file_suffix", s.encrypted_copy_file_suffix),
            (
                "encrypted_copy_same_file_title",
                s.encrypted_copy_same_file_title,
            ),
            (
                "encrypted_copy_same_file_message",
                s.encrypted_copy_same_file_message,
            ),
            ("decrypted_copy_dialog_title", s.decrypted_copy_dialog_title),
            ("decrypted_copy_file_suffix", s.decrypted_copy_file_suffix),
            (
                "decrypted_copy_same_file_title",
                s.decrypted_copy_same_file_title,
            ),
            (
                "decrypted_copy_same_file_message",
                s.decrypted_copy_same_file_message,
            ),
            (
                "compressed_copy_dialog_title",
                s.compressed_copy_dialog_title,
            ),
            ("compressed_copy_file_suffix", s.compressed_copy_file_suffix),
            (
                "compressed_copy_same_file_title",
                s.compressed_copy_same_file_title,
            ),
            (
                "compressed_copy_same_file_message",
                s.compressed_copy_same_file_message,
            ),
            ("exif_cleaned_file_suffix", s.exif_cleaned_file_suffix),
            ("watermark_file_suffix", s.watermark_file_suffix),
            ("page_marks_dialog_title", s.page_marks_dialog_title),
            ("page_marks_file_suffix", s.page_marks_file_suffix),
            ("page_marks_same_file_title", s.page_marks_same_file_title),
            (
                "page_marks_same_file_message",
                s.page_marks_same_file_message,
            ),
            (
                "searchable_copy_dialog_title",
                s.searchable_copy_dialog_title,
            ),
            ("searchable_copy_file_suffix", s.searchable_copy_file_suffix),
            (
                "searchable_copy_same_file_title",
                s.searchable_copy_same_file_title,
            ),
            (
                "searchable_copy_same_file_message",
                s.searchable_copy_same_file_message,
            ),
            ("certificate_dialog_title", s.certificate_dialog_title),
            ("certificate_filter_name", s.certificate_filter_name),
            ("signed_copy_dialog_title", s.signed_copy_dialog_title),
            ("signed_copy_file_suffix", s.signed_copy_file_suffix),
            ("signed_copy_same_file_title", s.signed_copy_same_file_title),
            (
                "signed_copy_same_file_message",
                s.signed_copy_same_file_message,
            ),
            ("signature_signed_by_prefix", s.signature_signed_by_prefix),
            ("signature_date_prefix", s.signature_date_prefix),
            ("signature_utc_open", s.signature_utc_open),
            ("signature_utc_close", s.signature_utc_close),
            ("signature_reason_prefix", s.signature_reason_prefix),
            ("create_sources_dialog_title", s.create_sources_dialog_title),
            ("create_all_filter_name", s.create_all_filter_name),
            ("create_images_filter_name", s.create_images_filter_name),
            ("create_html_filter_name", s.create_html_filter_name),
            ("create_text_filter_name", s.create_text_filter_name),
            ("create_books_filter_name", s.create_books_filter_name),
            ("create_svg_filter_name", s.create_svg_filter_name),
            ("create_office_filter_name", s.create_office_filter_name),
            ("create_pdf_dialog_title", s.create_pdf_dialog_title),
            ("create_pdf_same_file_title", s.create_pdf_same_file_title),
            (
                "create_pdf_same_file_message",
                s.create_pdf_same_file_message,
            ),
            (
                "explorer_create_too_many_title",
                s.explorer_create_too_many_title,
            ),
            (
                "explorer_create_too_many_prefix",
                s.explorer_create_too_many_prefix,
            ),
            (
                "explorer_create_too_many_suffix",
                s.explorer_create_too_many_suffix,
            ),
            (
                "explorer_create_skipped_title",
                s.explorer_create_skipped_title,
            ),
            (
                "explorer_create_skipped_files_one",
                s.explorer_create_skipped_files_one,
            ),
            (
                "explorer_create_skipped_files_other_suffix",
                s.explorer_create_skipped_files_other_suffix,
            ),
            (
                "explorer_create_skipped_prefix",
                s.explorer_create_skipped_prefix,
            ),
            (
                "explorer_create_skipped_middle",
                s.explorer_create_skipped_middle,
            ),
            (
                "explorer_create_skipped_images_one",
                s.explorer_create_skipped_images_one,
            ),
            (
                "explorer_create_skipped_images_other_suffix",
                s.explorer_create_skipped_images_other_suffix,
            ),
            (
                "explorer_create_skipped_suffix",
                s.explorer_create_skipped_suffix,
            ),
            (
                "explorer_create_all_skipped_title",
                s.explorer_create_all_skipped_title,
            ),
            (
                "explorer_create_all_skipped_prefix",
                s.explorer_create_all_skipped_prefix,
            ),
            (
                "explorer_create_all_skipped_suffix",
                s.explorer_create_all_skipped_suffix,
            ),
            ("language_dialog_title", s.language_dialog_title),
            ("language_filter_name", s.language_filter_name),
            ("font_dialog_title", s.font_dialog_title),
            ("font_filter_name", s.font_filter_name),
            ("export_text_dialog_title", s.export_text_dialog_title),
            (
                "export_annotations_dialog_title",
                s.export_annotations_dialog_title,
            ),
            ("markdown_filter_name", s.markdown_filter_name),
            (
                "annotation_summary_file_suffix",
                s.annotation_summary_file_suffix,
            ),
            ("annotation_summary_colon", s.annotation_summary_colon),
            ("annotation_summary_pipe", s.annotation_summary_pipe),
            (
                "annotation_summary_truncated",
                s.annotation_summary_truncated,
            ),
            ("export_xfdf_dialog_title", s.export_xfdf_dialog_title),
            ("xfdf_filter_name", s.xfdf_filter_name),
            ("xfdf_file_suffix", s.xfdf_file_suffix),
            ("kind_highlight", s.kind_highlight),
            ("kind_underline", s.kind_underline),
            ("kind_strike_out", s.kind_strike_out),
            ("kind_squiggly", s.kind_squiggly),
            ("kind_note", s.kind_note),
            ("kind_text_box", s.kind_text_box),
            ("kind_ink", s.kind_ink),
            ("kind_stamp", s.kind_stamp),
            ("kind_rectangle", s.kind_rectangle),
            ("kind_ellipse", s.kind_ellipse),
            ("kind_line", s.kind_line),
            ("kind_arrow", s.kind_arrow),
            ("export_compare_dialog_title", s.export_compare_dialog_title),
            ("compare_joiner", s.compare_joiner),
            ("compare_file_suffix", s.compare_file_suffix),
            ("compare_title_suffix", s.compare_title_suffix),
            ("compare_name_open", s.compare_name_open),
            ("compare_name_mid", s.compare_name_mid),
            ("compare_name_close", s.compare_name_close),
            ("compare_label_deleted", s.compare_label_deleted),
            ("compare_label_inserted", s.compare_label_inserted),
            ("compare_colon", s.compare_colon),
            ("compare_identical", s.compare_identical),
            ("compare_empty", s.compare_empty),
            ("compare_no_text_open", s.compare_no_text_open),
            ("compare_no_text_close", s.compare_no_text_close),
            ("compare_truncated", s.compare_truncated),
            ("compare_page_text_truncated", s.compare_page_text_truncated),
            ("compare_recognised_note", s.compare_recognised_note),
            ("export_docx_dialog_title", s.export_docx_dialog_title),
            ("docx_filter_name", s.docx_filter_name),
            ("text_filter_name", s.text_filter_name),
            ("export_images_dialog_title", s.export_images_dialog_title),
            ("stamp_image_dialog_title", s.stamp_image_dialog_title),
            (
                "watermark_image_dialog_title",
                s.watermark_image_dialog_title,
            ),
            ("image_filter_name", s.image_filter_name),
            ("split_dialog_title", s.split_dialog_title),
            ("split_same_file_title", s.split_same_file_title),
            ("split_same_file_message", s.split_same_file_message),
            ("split_selected_suffix", s.split_selected_suffix),
            ("split_folder_dialog_title", s.split_folder_dialog_title),
            ("batch_files_dialog_title", s.batch_files_dialog_title),
            ("batch_folder_dialog_title", s.batch_folder_dialog_title),
            ("batch_tray_tooltip_open", s.batch_tray_tooltip_open),
            ("batch_tray_tooltip_mid", s.batch_tray_tooltip_mid),
            ("batch_tray_tooltip_close", s.batch_tray_tooltip_close),
            ("batch_summary_done_prefix", s.batch_summary_done_prefix),
            ("batch_summary_failed_prefix", s.batch_summary_failed_prefix),
            (
                "batch_summary_skipped_prefix",
                s.batch_summary_skipped_prefix,
            ),
            ("batch_summary_separator", s.batch_summary_separator),
            ("batch_summary_end", s.batch_summary_end),
            ("batch_tray_show", s.batch_tray_show),
            ("batch_tray_stop", s.batch_tray_stop),
            ("batch_done_title", s.batch_done_title),
            ("batch_done_report_prefix", s.batch_done_report_prefix),
            ("batch_done_report_failed", s.batch_done_report_failed),
            ("batch_report_file_name", s.batch_report_file_name),
            ("batch_report_title", s.batch_report_title),
            (
                "batch_report_operation_prefix",
                s.batch_report_operation_prefix,
            ),
            ("batch_report_folder_prefix", s.batch_report_folder_prefix),
            ("batch_op_remove_metadata", s.batch_op_remove_metadata),
            ("batch_op_remove_exif", s.batch_op_remove_exif),
            ("batch_op_encrypt", s.batch_op_encrypt),
            ("batch_op_watermark", s.batch_op_watermark),
            ("batch_op_compress_images", s.batch_op_compress_images),
            ("batch_op_sanitize", s.batch_op_sanitize),
            ("batch_op_page_marks", s.batch_op_page_marks),
            ("bates_range_separator", s.bates_range_separator),
            ("batch_not_done", s.batch_not_done),
            ("batch_outcome_done_prefix", s.batch_outcome_done_prefix),
            ("batch_outcome_bates_open", s.batch_outcome_bates_open),
            ("batch_outcome_bates_close", s.batch_outcome_bates_close),
            ("batch_outcome_failed_prefix", s.batch_outcome_failed_prefix),
            ("batch_outcome_skipped", s.batch_outcome_skipped),
            ("reason_encrypted", s.reason_encrypted),
            ("reason_invalid_argument", s.reason_invalid_argument),
            ("reason_signed", s.reason_signed),
            ("reason_bates_overflow", s.reason_bates_overflow),
            ("reason_no_page_to_mark", s.reason_no_page_to_mark),
            ("reason_not_pdf", s.reason_not_pdf),
            ("reason_corrupted", s.reason_corrupted),
            (
                "reason_unsupported_encryption",
                s.reason_unsupported_encryption,
            ),
            ("reason_unreadable", s.reason_unreadable),
            ("reason_too_large", s.reason_too_large),
            ("reason_limit_exceeded", s.reason_limit_exceeded),
            ("reason_worker_error", s.reason_worker_error),
            ("reason_read_only", s.reason_read_only),
            ("reason_disk_full", s.reason_disk_full),
            ("reason_unwritable", s.reason_unwritable),
            ("reason_internal", s.reason_internal),
            ("overwrite_title", s.overwrite_title),
            ("overwrite_one", s.overwrite_one),
            ("overwrite_many_prefix", s.overwrite_many_prefix),
            ("overwrite_many_suffix", s.overwrite_many_suffix),
            ("no_text_layer_page", s.no_text_layer_page),
            ("webview2_missing_title", s.webview2_missing_title),
            ("webview2_missing_message", s.webview2_missing_message),
        ];
        for (name, val) in required_fields {
            check_en_text(name, val, false);
        }
        for (name, val) in [
            (
                "page_number_parts.page_after",
                s.page_number_parts.page_after,
            ),
            (
                "page_number_parts.total_after",
                s.page_number_parts.total_after,
            ),
            ("batch_summary_done_suffix", s.batch_summary_done_suffix),
            ("batch_summary_failed_suffix", s.batch_summary_failed_suffix),
            (
                "batch_summary_skipped_suffix",
                s.batch_summary_skipped_suffix,
            ),
        ] {
            check_en_text(name, val, true);
        }

        // Parameterized methods on Strings
        let time = ipc_contract::worker::SigningTime {
            year: 2026,
            month: 10,
            day: 22,
            hour: 9,
            minute: 5,
            second: 0,
            utc_offset_minutes: 480,
        };
        for line in s.signature_lines("Alice", &time, Some("Approved")) {
            check_en_text("signature_lines", &line, false);
        }
        for sample in [
            s.window_title(Some("doc.pdf"), true),
            s.window_title(None, false),
            s.privacy_export_file_name("report"),
            s.sanitized_copy_file_name("report"),
            s.encrypted_copy_file_name("report"),
            s.decrypted_copy_file_name("report"),
            s.compressed_copy_file_name("report"),
            s.exif_cleaned_file_name("report"),
            s.watermark_file_name("report"),
            s.page_marks_file_name("report"),
            s.searchable_copy_file_name("report"),
            s.signed_copy_file_name("report"),
            s.created_pdf_file_name("report"),
            s.explorer_create_too_many_message(200),
            s.explorer_create_skipped_message(1, 1),
            s.explorer_create_skipped_message(2, 3),
            s.explorer_create_all_skipped_message(1),
            s.explorer_create_all_skipped_message(4),
            s.annotation_summary_file_name("report", true),
            s.xfdf_file_name("report"),
            s.annotation_summary_page(3),
            s.compare_file_name("a", "b", true),
            s.compare_title("a.pdf", "b.pdf"),
            s.compare_item_header(1, "a.pdf", 2, "b.pdf"),
            s.compare_no_text_one_side("a.pdf"),
            s.batch_tray_tooltip(1, 5),
            s.batch_done_message("Done 1.", Some(r"C:\out\report.txt")),
            s.batch_done_message("Done 1.", None),
            s.batch_report_operation("Encrypt"),
            s.batch_report_folder(r"C:\out"),
            s.bates_range("CASE-000001", "CASE-000010"),
            s.batch_summary(1, 2, 3),
            s.batch_outcome(&BatchOutcome::Done {
                output: "out.pdf".to_owned(),
                bates: None,
            }),
            s.batch_outcome(&BatchOutcome::Skipped),
            s.split_file_name("report", &[1, 3]),
            s.split_part_name("report", &[0, 2], 1, 5),
            s.overwrite_message(1),
            s.overwrite_message(3),
        ] {
            check_en_text("method output", &sample, false);
        }
        for code in [
            ErrorCode::Encrypted,
            ErrorCode::InvalidArgument,
            ErrorCode::Signed,
            ErrorCode::BatesOverflow,
            ErrorCode::NoPageToMark,
            ErrorCode::NotPdf,
            ErrorCode::Corrupted,
            ErrorCode::UnsupportedEncryption,
            ErrorCode::Unreadable,
            ErrorCode::TooLarge,
            ErrorCode::LimitExceeded,
            ErrorCode::WorkerCrashed,
            ErrorCode::ReadOnly,
            ErrorCode::DiskFull,
            ErrorCode::Unwritable,
            ErrorCode::Internal,
        ] {
            check_en_text(
                "batch_outcome(Failed)",
                &s.batch_outcome(&BatchOutcome::Failed { error: code }),
                false,
            );
        }
    }

    #[test]
    fn english_strings_have_no_chinese_characters_or_empty_values() {
        check_struct_strings(&EN);
        assert!(std::ptr::eq(for_language("en"), &EN));
        assert!(std::ptr::eq(for_language("zh-TW"), &ZH_TW));
    }

    #[test]
    fn an_xfdf_export_is_named_after_the_document() {
        assert_eq!(ZH_TW.xfdf_file_name("報告"), "報告（註解）.xfdf");
        assert_eq!(ZH_TW.xfdf_file_name("file_name"), "file_name（註解）.xfdf");
        assert_eq!(
            ZH_TW.searchable_copy_file_name("報告"),
            "報告（可搜尋）.pdf"
        );
        assert_eq!(
            ZH_TW.searchable_copy_file_name("scan"),
            "scan（可搜尋）.pdf"
        );

        assert_eq!(EN.xfdf_file_name("report"), "report (annotations).xfdf");
        assert_eq!(
            EN.searchable_copy_file_name("scan"),
            "scan (searchable).pdf"
        );
    }

    #[test]
    fn explorer_create_messages_state_counts_and_limits() {
        for s in [&ZH_TW, &EN] {
            assert!(s.explorer_create_too_many_message(200).contains("200"));
            let skipped = s.explorer_create_skipped_message(2, 3);
            assert!(skipped.contains("2") && skipped.contains("3"));
            let all_skipped = s.explorer_create_all_skipped_message(4);
            assert!(all_skipped.contains("4"));
        }
    }

    #[test]
    fn an_annotation_summary_is_named_after_the_document_and_its_kind() {
        assert_eq!(
            ZH_TW.annotation_summary_file_name("報告", true),
            "報告（註解摘要）.md"
        );
        assert_eq!(
            ZH_TW.annotation_summary_file_name("報告", false),
            "報告（註解摘要）.txt"
        );
        assert_eq!(
            EN.annotation_summary_file_name("report", true),
            "report (annotation summary).md"
        );
        assert_eq!(
            EN.annotation_summary_file_name("report", false),
            "report (annotation summary).txt"
        );
    }

    #[test]
    fn compare_file_name_formats() {
        assert_eq!(
            ZH_TW.compare_file_name("合約A", "合約B", true),
            "合約A 與 合約B（文字比較）.md"
        );
        assert_eq!(
            ZH_TW.compare_file_name("合約A", "合約B", false),
            "合約A 與 合約B（文字比較）.txt"
        );
        assert_eq!(
            EN.compare_file_name("v1", "v2", true),
            "v1 vs v2 (text comparison).md"
        );
        assert_eq!(
            EN.compare_file_name("v1", "v2", false),
            "v1 vs v2 (text comparison).txt"
        );
    }

    #[test]
    fn split_files_are_named_by_their_pages_or_by_their_number() {
        assert_eq!(ZH_TW.split_file_name("報告", &[1, 2, 3]), "報告-p2-4.pdf");
        assert_eq!(ZH_TW.split_file_name("報告", &[4]), "報告-p5.pdf");
        assert_eq!(
            ZH_TW.split_file_name("報告", &[1, 3]),
            "報告（選取的頁面）.pdf"
        );
        assert_eq!(
            ZH_TW.split_file_name("報告", &[2, 1]),
            "報告（選取的頁面）.pdf"
        );
        assert_eq!(
            ZH_TW.split_part_name("報告", &[0, 1], 0, 3),
            "報告-p1-2.pdf"
        );
        // Pages that do not follow one another: the number of the part, as wide as the last.
        assert_eq!(ZH_TW.split_part_name("報告", &[0, 2], 1, 12), "報告-02.pdf");
        assert_eq!(ZH_TW.split_part_name("報告", &[0, 2], 1, 5), "報告-2.pdf");

        assert_eq!(
            EN.split_file_name("report", &[1, 3]),
            "report (selected pages).pdf"
        );
        assert_eq!(EN.split_file_name("report", &[1, 2, 3]), "report-p2-4.pdf");
    }

    #[test]
    fn the_window_title_names_the_open_file_and_marks_unsaved_changes() {
        for s in [&ZH_TW, &EN] {
            assert_eq!(
                s.window_title(Some("report.pdf"), false),
                "report.pdf — PDF Reader"
            );
            assert_eq!(
                s.window_title(Some("report.pdf"), true),
                "• report.pdf — PDF Reader"
            );
            assert_eq!(s.window_title(None, false), "PDF Reader");
        }
    }

    #[test]
    fn visible_signature_lines_match_language() {
        let time = ipc_contract::worker::SigningTime {
            year: 2026,
            month: 10,
            day: 22,
            hour: 9,
            minute: 5,
            second: 0,
            utc_offset_minutes: 480,
        };
        assert_eq!(
            ZH_TW.signature_lines("王小明", &time, Some("同意")),
            [
                "數位簽署：王小明",
                "日期：2026-10-22 09:05（UTC+08:00）",
                "原因：同意",
            ]
        );
        assert_eq!(
            EN.signature_lines("Alice", &time, Some("Approved")),
            [
                "Digitally signed by: Alice",
                "Date: 2026-10-22 09:05 (UTC+08:00)",
                "Reason: Approved",
            ]
        );
    }

    #[test]
    fn a_batch_says_that_a_signed_file_would_lose_its_signatures() {
        let sanitize = BatchOperation::Sanitize {
            options: Default::default(),
        };
        assert_eq!(ZH_TW.batch_operation_name(&sanitize), "分享前淨化");
        assert_eq!(
            EN.batch_operation_name(&sanitize),
            "Sanitize before sharing"
        );
        assert_eq!(
            ZH_TW.batch_outcome(&BatchOutcome::Failed {
                error: ErrorCode::Signed
            }),
            "失敗：有數位簽章的檔案，這樣處理會讓簽章失效"
        );
        assert_eq!(
            EN.batch_outcome(&BatchOutcome::Failed {
                error: ErrorCode::Signed
            }),
            "Failed: Digitally signed file; processing would invalidate signature"
        );
        let page_marks = BatchOperation::PageMarks {
            marks: Box::new(ipc_contract::types::PageMarks {
                page_number: None,
                bates: None,
                header_footer: None,
                font_size: 10.0,
                color: ipc_contract::types::Rgb { r: 0, g: 0, b: 0 },
                margin_pt: 36.0,
                pages: ipc_contract::types::PageMarkPages::All,
            }),
        };
        assert_eq!(ZH_TW.batch_operation_name(&page_marks), "頁碼與 Bates 編號");
        assert_eq!(
            EN.batch_operation_name(&page_marks),
            "Page numbers and Bates numbering"
        );
        assert_eq!(
            ZH_TW.bates_range("CASE-000001", "CASE-000010"),
            "CASE-000001～CASE-000010"
        );
        assert_eq!(
            EN.bates_range("CASE-000001", "CASE-000010"),
            "CASE-000001–CASE-000010"
        );
        assert_eq!(ZH_TW.bates_range("ABC0005", "ABC0005"), "ABC0005");
        assert_eq!(
            ZH_TW.batch_outcome(&BatchOutcome::Done {
                output: "報告.pdf".to_owned(),
                bates: Some(ipc_contract::types::BatesRange {
                    first: "CASE-000001".to_owned(),
                    last: "CASE-000010".to_owned(),
                }),
            }),
            "完成：報告.pdf（CASE-000001～CASE-000010）"
        );
        assert_eq!(
            EN.batch_outcome(&BatchOutcome::Done {
                output: "report.pdf".to_owned(),
                bates: Some(ipc_contract::types::BatesRange {
                    first: "CASE-000001".to_owned(),
                    last: "CASE-000010".to_owned(),
                }),
            }),
            "Done: report.pdf (CASE-000001–CASE-000010)"
        );
        assert_eq!(
            ZH_TW.batch_outcome(&BatchOutcome::Done {
                output: "報告.pdf".to_owned(),
                bates: Some(ipc_contract::types::BatesRange {
                    first: "CASE-000001".to_owned(),
                    last: "CASE-000001".to_owned(),
                }),
            }),
            "完成：報告.pdf（CASE-000001）"
        );
        assert_eq!(
            ZH_TW.batch_outcome(&BatchOutcome::Done {
                output: "報告.pdf".to_owned(),
                bates: None,
            }),
            "完成：報告.pdf"
        );
        assert_eq!(
            ZH_TW.batch_outcome(&BatchOutcome::Failed {
                error: ErrorCode::BatesOverflow
            }),
            "失敗：這個檔案的 Bates 編號超出位數上限"
        );
        assert_eq!(
            ZH_TW.batch_outcome(&BatchOutcome::Failed {
                error: ErrorCode::NoPageToMark
            }),
            "失敗：這個檔案沒有要加標記的頁面，或頁面範圍超出它的頁數"
        );
    }

    /// Scans non-test Rust source string literals in `dir` and returns `(path, line, literal)`
    /// violations containing Han characters. If `allow_zh_tw_const` is true (`strings.rs`), the
    /// `pub const ZH_TW: Strings = Strings { ... };` initializer is exempted.
    pub(crate) fn scan_non_test_hanzi_string_literals(
        source: &str,
        allow_zh_tw_const: bool,
    ) -> Vec<(usize, String)> {
        let mut violations = Vec::new();
        let bytes = source.as_bytes();
        let mut i = 0usize;
        let mut line = 1usize;
        let mut depth = 0i32;

        // Pending #[cfg(test)] item: wait for its `{ ... }` (or `;`), then ignore until depth returns.
        let mut pending_test_item = false;
        let mut skip_test_depth: Option<i32> = None;
        let mut skip_zh_tw_depth: Option<i32> = None;

        while i < bytes.len() {
            if bytes[i] == b'\n' {
                line += 1;
                i += 1;
                continue;
            }
            // Comments
            if bytes[i..].starts_with(b"//") {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i..].starts_with(b"/*") {
                i += 2;
                let mut comment_depth = 1usize;
                while i < bytes.len() && comment_depth > 0 {
                    if bytes[i] == b'\n' {
                        line += 1;
                        i += 1;
                    } else if bytes[i..].starts_with(b"/*") {
                        comment_depth += 1;
                        i += 2;
                    } else if bytes[i..].starts_with(b"*/") {
                        comment_depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
            // Attributes: detect `#[cfg(test)]` and `#[cfg(all(test, ...))]`
            if bytes[i..].starts_with(b"#[") && skip_test_depth.is_none() {
                let start = i;
                i += 2;
                let mut attr_depth = 1usize;
                while i < bytes.len() && attr_depth > 0 {
                    if bytes[i] == b'\n' {
                        line += 1;
                    } else if bytes[i] == b'[' {
                        attr_depth += 1;
                    } else if bytes[i] == b']' {
                        attr_depth -= 1;
                    }
                    i += 1;
                }
                if is_cfg_test_attribute(&source[start..i]) {
                    pending_test_item = true;
                }
                continue;
            }
            // Raw strings: r"..." / r#"..."# / br#"..."#
            if let Some((content_start, content_end, end_pos, literal_line)) =
                parse_raw_string(source, i, line)
            {
                let content = &source[content_start..content_end];
                line += content.bytes().filter(|&b| b == b'\n').count();
                if skip_test_depth.is_none()
                    && skip_zh_tw_depth.is_none()
                    && content.chars().any(is_hanzi)
                {
                    violations.push((literal_line, content.to_owned()));
                }
                i = end_pos;
                continue;
            }
            // Normal strings: "..." or b"..."
            if bytes[i] == b'"' || (bytes[i..].starts_with(b"b\"") && !is_ident_before(bytes, i)) {
                let quote_at = if bytes[i] == b'"' { i } else { i + 1 };
                let literal_line = line;
                i = quote_at + 1;
                let content_start = i;
                while i < bytes.len() {
                    if bytes[i] == b'\\' {
                        i += 1;
                        if i < bytes.len() {
                            if bytes[i] == b'\n' {
                                line += 1;
                            }
                            i += 1;
                        }
                        continue;
                    }
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    if bytes[i] == b'"' {
                        break;
                    }
                    i += 1;
                }
                let content = &source[content_start..i];
                if skip_test_depth.is_none()
                    && skip_zh_tw_depth.is_none()
                    && content.chars().any(is_hanzi)
                {
                    violations.push((literal_line, content.to_owned()));
                }
                if i < bytes.len() {
                    i += 1;
                }
                continue;
            }
            // Char literal vs lifetime
            if bytes[i] == b'\''
                && let Some(end_pos) = parse_char_literal(bytes, i)
            {
                i = end_pos;
                continue;
            }
            // Detect `pub static ZH_TW: Strings`
            if allow_zh_tw_const
                && skip_test_depth.is_none()
                && skip_zh_tw_depth.is_none()
                && source[i..].starts_with("pub static ZH_TW:")
                && let Some(brace_rel) = source[i..].find('{')
            {
                // Advance to the `{` cleanly while tracking newlines.
                let slice = &source[i..i + brace_rel];
                line += slice.bytes().filter(|&b| b == b'\n').count();
                i += brace_rel;
                skip_zh_tw_depth = Some(depth);
                depth += 1;
                i += 1;
                continue;
            }
            if bytes[i] == b'{' {
                if pending_test_item {
                    skip_test_depth = Some(depth);
                    pending_test_item = false;
                }
                depth += 1;
            } else if bytes[i] == b'}' {
                depth -= 1;
                if skip_test_depth == Some(depth) {
                    skip_test_depth = None;
                }
                if skip_zh_tw_depth == Some(depth) {
                    skip_zh_tw_depth = None;
                }
            } else if bytes[i] == b';' && pending_test_item {
                pending_test_item = false;
            }
            i += 1;
        }
        violations
    }

    fn is_ident_before(bytes: &[u8], i: usize) -> bool {
        i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')
    }

    fn parse_raw_string(
        source: &str,
        i: usize,
        line: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        let bytes = source.as_bytes();
        let mut cursor = i;
        if bytes[cursor] == b'b' && cursor + 1 < bytes.len() && bytes[cursor + 1] == b'r' {
            if is_ident_before(bytes, i) {
                return None;
            }
            cursor += 2;
        } else if bytes[cursor] == b'r' {
            if is_ident_before(bytes, i) {
                return None;
            }
            cursor += 1;
        } else {
            return None;
        }
        let mut hashes = 0usize;
        while cursor < bytes.len() && bytes[cursor] == b'#' {
            hashes += 1;
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'"' {
            return None;
        }
        cursor += 1;
        let content_start = cursor;
        while cursor < bytes.len() {
            if bytes[cursor] == b'"' {
                let after_quote = cursor + 1;
                if after_quote + hashes <= bytes.len()
                    && bytes[after_quote..after_quote + hashes]
                        .iter()
                        .all(|&b| b == b'#')
                {
                    return Some((content_start, cursor, after_quote + hashes, line));
                }
            }
            cursor += 1;
        }
        Some((content_start, bytes.len(), bytes.len(), line))
    }

    fn parse_char_literal(bytes: &[u8], i: usize) -> Option<usize> {
        // Distinguish `'x'` / `'\n'` / `'\u{1234}'` / `'\''` from `'a` lifetimes.
        if i + 1 >= bytes.len() {
            return None;
        }
        if bytes[i + 1] == b'\\' {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b'\'' && bytes[j] != b'\n' && j - i < 16 {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'\'' {
                return Some(j + 1);
            }
            return None;
        }
        let rest = std::str::from_utf8(&bytes[i + 1..]).ok()?;
        let mut chars = rest.chars();
        let ch = chars.next()?;
        if ch != '\'' && ch != '\n' && chars.next() == Some('\'') {
            return Some(i + 1 + ch.len_utf8() + 1);
        }
        None
    }

    fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn non_test_main_process_and_worker_source_has_no_hardcoded_chinese_strings() {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        collect_rs_files(&manifest.join("src"), &mut files);
        collect_rs_files(&manifest.join("../crates/pdf_worker/src"), &mut files);
        let mut found = Vec::new();
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            let is_strings_rs = file.file_name().and_then(|n| n.to_str()) == Some("strings.rs");
            for (line, lit) in scan_non_test_hanzi_string_literals(&source, is_strings_rs) {
                found.push(format!("{}:{}: {:?}", file.display(), line, lit));
            }
        }
        assert!(
            found.is_empty(),
            "hardcoded Chinese string literals found outside strings::ZH_TW / #[cfg(test)]:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn hanzi_source_scanner_handles_comments_raw_strings_and_cfg_test() {
        let sample = r###"
            // 中文註解可以存在
            /* 巢狀 /* 中文 */ 註解也可以 */
            fn ok<'a>(c: char) -> bool { c == '中' || c == '\'' }
            fn bad_one() -> &'static str { "這裡不能寫中文" }
            fn bad_two() -> &'static str { r#"也不能「寫死」中文"# }
            #[cfg(test)]
            mod tests {
                const OK_IN_TEST: &str = "測試裡可以有中文";
            }
        "###;
        let hits = scan_non_test_hanzi_string_literals(sample, false);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].1, "這裡不能寫中文");
        assert_eq!(hits[1].1, "也不能「寫死」中文");

        let _ = (is_cfg_test_attribute, collect_rs_files);
        let strings_sample = r#"
            pub static ZH_TW: Strings = Strings { title: "允許中文" };
            pub const OTHER: &str = "不該在外面";
        "#;
        let hits = scan_non_test_hanzi_string_literals(strings_sample, true);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].1, "不該在外面");

        let cfg_all_sample = r#"
            #[cfg(all(test, windows))]
            mod winrt_tests {
                const OK: &str = "測試裡可以有中文";
            }
            #[cfg(not(test))]
            const BAD_PROD: &str = "正式環境不該寫死中文";
        "#;
        let hits = scan_non_test_hanzi_string_literals(cfg_all_sample, false);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].1, "正式環境不該寫死中文");
    }

    fn is_cfg_test_attribute(attr: &str) -> bool {
        let compact: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
        let Some(inner) = compact
            .strip_prefix("#[cfg(")
            .and_then(|rest| rest.strip_suffix(")]"))
        else {
            return false;
        };
        if inner.contains("not(test)") {
            return false;
        }
        inner
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .any(|token| token == "test")
    }
}
