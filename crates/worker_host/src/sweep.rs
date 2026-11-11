//! Public PDF corpus robustness scanner (Issue #439, B5-02).
//!
//! Sweeps untrusted PDF files through the sandboxed `pdf_worker` without parsing
//! them in the main process. Renders each page and extracts text, detecting crashes,
//! timeouts, protocol violations, or rendering/extraction failures.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc_contract::raster::fit_scale;
use ipc_contract::types::Rotation;
use ipc_contract::worker::{WorkerRequest, WorkerResponse};

use crate::{HostConfig, HostError, WorkerHost, bundled_worker_path};

/// Escapes a string for valid JSON output.
pub fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// The result category of scanning a single PDF file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SweepResultKind {
    WorkerCrashed,
    ProtocolViolation,
    WorkerTimeout,
    FileTimeout,
    RenderFailed,
    GetTextFailed,
    OpenFailed,
    Unreadable,
    Ok,
}

impl SweepResultKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkerCrashed => "worker_crashed",
            Self::ProtocolViolation => "protocol_violation",
            Self::WorkerTimeout => "worker_timeout",
            Self::FileTimeout => "file_timeout",
            Self::RenderFailed => "render_failed",
            Self::GetTextFailed => "get_text_failed",
            Self::OpenFailed => "open_failed",
            Self::Unreadable => "unreadable",
            Self::Ok => "ok",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::WorkerCrashed => "worker 當掉 (crashed)",
            Self::ProtocolViolation => "協定錯誤 (protocol violation)",
            Self::WorkerTimeout => "worker 逾時 (worker timeout)",
            Self::FileTimeout => "檔案時間用完 (file timeout)",
            Self::RenderFailed => "渲染失敗 (render failed)",
            Self::GetTextFailed => "擷取文字失敗 (text failed)",
            Self::OpenFailed => "開啟失敗 (open failed)",
            Self::Unreadable => "無法讀取 (unreadable)",
            Self::Ok => "正常 (ok)",
        }
    }

    /// Whether this is a notable problem that should be listed in the issues table.
    pub fn is_problem(&self) -> bool {
        matches!(
            self,
            Self::WorkerCrashed
                | Self::ProtocolViolation
                | Self::WorkerTimeout
                | Self::FileTimeout
                | Self::RenderFailed
                | Self::GetTextFailed
        )
    }
}

/// The stage at which the scan stopped or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepStage {
    Open,
    Render,
    GetText,
    ReadFile,
    FileTimeout,
}

impl SweepStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Render => "render",
            Self::GetText => "get_text",
            Self::ReadFile => "read_file",
            Self::FileTimeout => "file_timeout",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Open => "開啟 (open)",
            Self::Render => "渲染頁面 (render)",
            Self::GetText => "擷取文字 (get_text)",
            Self::ReadFile => "讀檔 (read_file)",
            Self::FileTimeout => "檔案時間用完 (file_timeout)",
        }
    }
}

/// Result of scanning one PDF file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResult {
    pub corpus: String,
    pub relative_path: String,
    pub bytes: u64,
    pub pages: Option<u32>,
    pub result: SweepResultKind,
    pub stage: SweepStage,
    pub page_index: Option<u32>,
    pub error_code: Option<String>,
    pub elapsed_ms: u64,
    pub peak_memory_bytes: Option<usize>,
}

impl FileResult {
    pub fn to_json_line(&self) -> String {
        format!(
            "{{\"corpus\":\"{}\",\"relative_path\":\"{}\",\"bytes\":{},\"pages\":{},\"result\":\"{}\",\"stage\":\"{}\",\"page_index\":{},\"error_code\":{},\"elapsed_ms\":{},\"peak_memory_bytes\":{}}}",
            escape_json(&self.corpus),
            escape_json(&self.relative_path),
            self.bytes,
            self.pages
                .map(|p| p.to_string())
                .unwrap_or_else(|| "null".into()),
            self.result.as_str(),
            self.stage.as_str(),
            self.page_index
                .map(|p| p.to_string())
                .unwrap_or_else(|| "null".into()),
            self.error_code
                .as_ref()
                .map(|c| format!("\"{}\"", escape_json(c)))
                .unwrap_or_else(|| "null".into()),
            self.elapsed_ms,
            self.peak_memory_bytes
                .map(|m| m.to_string())
                .unwrap_or_else(|| "null".into()),
        )
    }
}

/// A target PDF to sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepTarget {
    pub corpus: String,
    pub relative_path: String,
    pub full_path: PathBuf,
}

/// Statistics for one corpus.
#[derive(Debug, Default, Clone)]
pub struct CorpusStats {
    pub total: usize,
    pub ok: usize,
    pub open_failed: usize,
    pub render_failed: usize,
    pub get_text_failed: usize,
    pub worker_crashed: usize,
    pub worker_timeout: usize,
    pub protocol_violation: usize,
    pub file_timeout: usize,
    pub unreadable: usize,
    pub open_failed_codes: BTreeMap<String, usize>,
}

impl CorpusStats {
    pub fn record(&mut self, res: &FileResult) {
        self.total += 1;
        match res.result {
            SweepResultKind::Ok => self.ok += 1,
            SweepResultKind::OpenFailed => {
                self.open_failed += 1;
                let code = res.error_code.clone().unwrap_or_else(|| "Unknown".into());
                *self.open_failed_codes.entry(code).or_default() += 1;
            }
            SweepResultKind::RenderFailed => self.render_failed += 1,
            SweepResultKind::GetTextFailed => self.get_text_failed += 1,
            SweepResultKind::WorkerCrashed => self.worker_crashed += 1,
            SweepResultKind::WorkerTimeout => self.worker_timeout += 1,
            SweepResultKind::ProtocolViolation => self.protocol_violation += 1,
            SweepResultKind::FileTimeout => self.file_timeout += 1,
            SweepResultKind::Unreadable => self.unreadable += 1,
        }
    }
}

/// Aggregated summary of an entire sweep run.
#[derive(Debug, Default)]
pub struct SweepSummary {
    pub total_files: usize,
    pub elapsed_ms: u64,
    pub by_corpus: BTreeMap<String, CorpusStats>,
    pub notable_issues: Vec<FileResult>,
}

impl SweepSummary {
    pub fn from_results(results: &[FileResult], elapsed_ms: u64) -> Self {
        let mut summary = Self {
            total_files: results.len(),
            elapsed_ms,
            by_corpus: BTreeMap::new(),
            notable_issues: Vec::new(),
        };

        for res in results {
            summary
                .by_corpus
                .entry(res.corpus.clone())
                .or_default()
                .record(res);
            if res.result.is_problem() {
                summary.notable_issues.push(res.clone());
            }
        }

        // Sort notable issues by priority:
        // Crashed > ProtocolViolation > Timeout > FileTimeout > RenderFailed > GetTextFailed
        summary.notable_issues.sort_by(|a, b| {
            a.result
                .cmp(&b.result)
                .then_with(|| a.corpus.cmp(&b.corpus))
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });

        summary
    }

    /// Generates markdown summary report (`summary.md`).
    pub fn generate_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str("# 公開 PDF 語料庫穩健性掃描摘要 (Corpus Sweep Summary)\n\n");
        md.push_str(&format!(
            "- **掃描總檔案數**：{} 個\n- **總耗時**：{:.2} 秒\n\n",
            self.total_files,
            self.elapsed_ms as f64 / 1000.0
        ));

        // 1. Overview table
        md.push_str("## 各語料庫結果統計\n\n");
        md.push_str("| 語料 (Corpus) | 總數 | 正常 (OK) | 開啟失敗 | 渲染失敗 | 擷取文字失敗 | 當掉 (Crashed) | 逾時 (Timeout) | 協定錯誤 | 檔案時間用完 | 無法讀取 |\n");
        md.push_str("|---|---|---|---|---|---|---|---|---|---|---|\n");

        let mut tot_ok = 0;
        let mut tot_open_failed = 0;
        let mut tot_render_failed = 0;
        let mut tot_get_text_failed = 0;
        let mut tot_crashed = 0;
        let mut tot_timeout = 0;
        let mut tot_protocol = 0;
        let mut tot_file_timeout = 0;
        let mut tot_unreadable = 0;

        for (name, s) in &self.by_corpus {
            tot_ok += s.ok;
            tot_open_failed += s.open_failed;
            tot_render_failed += s.render_failed;
            tot_get_text_failed += s.get_text_failed;
            tot_crashed += s.worker_crashed;
            tot_timeout += s.worker_timeout;
            tot_protocol += s.protocol_violation;
            tot_file_timeout += s.file_timeout;
            tot_unreadable += s.unreadable;

            md.push_str(&format!(
                "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                name,
                s.total,
                s.ok,
                s.open_failed,
                s.render_failed,
                s.get_text_failed,
                s.worker_crashed,
                s.worker_timeout,
                s.protocol_violation,
                s.file_timeout,
                s.unreadable
            ));
        }

        md.push_str(&format!(
            "| **合計 (Total)** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** |\n\n",
            self.total_files,
            tot_ok,
            tot_open_failed,
            tot_render_failed,
            tot_get_text_failed,
            tot_crashed,
            tot_timeout,
            tot_protocol,
            tot_file_timeout,
            tot_unreadable
        ));

        // 2. Open failures distribution
        md.push_str("## 開啟失敗錯誤碼分佈 (Open Failure Codes)\n\n");
        let mut any_open_failures = false;
        for (name, s) in &self.by_corpus {
            if !s.open_failed_codes.is_empty() {
                any_open_failures = true;
                md.push_str(&format!("### 語料 `{name}`\n\n"));
                md.push_str("| 錯誤碼 (Error Code) | 數量 (Count) |\n|---|---|\n");
                for (code, count) in &s.open_failed_codes {
                    md.push_str(&format!("| `{code}` | {count} |\n"));
                }
                md.push('\n');
            }
        }
        if !any_open_failures {
            md.push_str("無開啟失敗之檔案。\n\n");
        }

        // 3. Notable issues list
        md.push_str("## 問題檔案清單 (Notable Issues)\n\n");
        md.push_str("> 依嚴重程度排列：Worker 當掉 > 協定錯誤 > Worker 逾時 > 檔案時間用完 > 渲染失敗 > 擷取文字失敗。\n\n");

        if self.notable_issues.is_empty() {
            md.push_str(
                "無（未發現任何 Worker 當掉、協定錯誤、逾時、渲染或擷取文字失敗之檔案）。\n",
            );
        } else {
            md.push_str(
                "| 語料 | 檔案路徑 | 結果種類 | 發生階段 | 頁碼 | 錯誤訊息 / 代碼 | 耗時 (ms) |\n",
            );
            md.push_str("|---|---|---|---|---|---|---|\n");
            for item in &self.notable_issues {
                let page_str = item
                    .page_index
                    .map(|p| format!("第 {} 頁", p + 1))
                    .unwrap_or_else(|| "-".into());
                let err_str = item.error_code.as_deref().unwrap_or("-").replace('\n', " ");
                md.push_str(&format!(
                    "| `{}` | `{}` | {} | {} | {} | `{}` | {} |\n",
                    item.corpus,
                    item.relative_path,
                    item.result.display_name(),
                    item.stage.display_name(),
                    page_str,
                    err_str,
                    item.elapsed_ms
                ));
            }
        }

        md
    }

    /// Generates structured JSON summary (`summary.json`).
    pub fn generate_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\n");
        out.push_str(&format!("  \"total_files\": {},\n", self.total_files));
        out.push_str(&format!("  \"elapsed_ms\": {},\n", self.elapsed_ms));
        out.push_str("  \"by_corpus\": {\n");

        let mut corpus_entries = Vec::new();
        for (name, s) in &self.by_corpus {
            let mut cstr = String::new();
            cstr.push_str(&format!("    \"{}\": {{\n", escape_json(name)));
            cstr.push_str(&format!("      \"total\": {},\n", s.total));
            cstr.push_str(&format!("      \"ok\": {},\n", s.ok));
            cstr.push_str(&format!("      \"open_failed\": {},\n", s.open_failed));
            cstr.push_str(&format!("      \"render_failed\": {},\n", s.render_failed));
            cstr.push_str(&format!(
                "      \"get_text_failed\": {},\n",
                s.get_text_failed
            ));
            cstr.push_str(&format!(
                "      \"worker_crashed\": {},\n",
                s.worker_crashed
            ));
            cstr.push_str(&format!(
                "      \"worker_timeout\": {},\n",
                s.worker_timeout
            ));
            cstr.push_str(&format!(
                "      \"protocol_violation\": {},\n",
                s.protocol_violation
            ));
            cstr.push_str(&format!("      \"file_timeout\": {},\n", s.file_timeout));
            cstr.push_str(&format!("      \"unreadable\": {},\n", s.unreadable));
            cstr.push_str("      \"open_failed_codes\": {");

            let mut code_entries = Vec::new();
            for (code, count) in &s.open_failed_codes {
                code_entries.push(format!("\"{}\": {}", escape_json(code), count));
            }
            if !code_entries.is_empty() {
                cstr.push_str(&format!(
                    "\n        {}\n      ",
                    code_entries.join(",\n        ")
                ));
            }
            cstr.push_str("}\n    }");
            corpus_entries.push(cstr);
        }

        out.push_str(&corpus_entries.join(",\n"));
        out.push_str("\n  },\n  \"by_result\": {\n");

        let mut ok = 0;
        let mut open_failed = 0;
        let mut render_failed = 0;
        let mut get_text_failed = 0;
        let mut worker_crashed = 0;
        let mut worker_timeout = 0;
        let mut protocol_violation = 0;
        let mut file_timeout = 0;
        let mut unreadable = 0;

        for s in self.by_corpus.values() {
            ok += s.ok;
            open_failed += s.open_failed;
            render_failed += s.render_failed;
            get_text_failed += s.get_text_failed;
            worker_crashed += s.worker_crashed;
            worker_timeout += s.worker_timeout;
            protocol_violation += s.protocol_violation;
            file_timeout += s.file_timeout;
            unreadable += s.unreadable;
        }

        out.push_str(&format!("    \"ok\": {ok},\n"));
        out.push_str(&format!("    \"open_failed\": {open_failed},\n"));
        out.push_str(&format!("    \"render_failed\": {render_failed},\n"));
        out.push_str(&format!("    \"get_text_failed\": {get_text_failed},\n"));
        out.push_str(&format!("    \"worker_crashed\": {worker_crashed},\n"));
        out.push_str(&format!("    \"worker_timeout\": {worker_timeout},\n"));
        out.push_str(&format!(
            "    \"protocol_violation\": {protocol_violation},\n"
        ));
        out.push_str(&format!("    \"file_timeout\": {file_timeout},\n"));
        out.push_str(&format!("    \"unreadable\": {unreadable}\n"));
        out.push_str("  }\n}\n");

        out
    }
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn collect_pdfs_recursive(
    dir: &Path,
    base_dir: &Path,
    corpus_name: &str,
    targets: &mut Vec<SweepTarget>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_pdfs_recursive(&path, base_dir, corpus_name, targets)?;
        } else if file_type.is_file() && is_pdf(&path) {
            let rel = path
                .strip_prefix(base_dir)
                .map_err(io::Error::other)?
                .to_string_lossy()
                .replace('\\', "/");
            targets.push(SweepTarget {
                corpus: corpus_name.to_string(),
                relative_path: rel,
                full_path: path,
            });
        }
    }
    Ok(())
}

/// Discovers PDF files to sweep in `corpus_dir`.
pub fn discover_files(
    corpus_dir: &Path,
    specific_corpus: Option<&str>,
) -> io::Result<Vec<SweepTarget>> {
    let mut targets = Vec::new();

    if let Some(corpus_name) = specific_corpus {
        collect_pdfs_recursive(corpus_dir, corpus_dir, corpus_name, &mut targets)?;
    } else {
        let mut subdirs = Vec::new();
        let mut root_pdfs = Vec::new();

        for entry in fs::read_dir(corpus_dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                subdirs.push(path);
            } else if file_type.is_file() && is_pdf(&path) {
                root_pdfs.push(path);
            }
        }

        // If subdirectories exist, treat each subdirectory with PDFs as a corpus
        let known_corpora = ["qpdf", "pdf-differences", "pdfjs"];
        let dir_name = corpus_dir.file_name().unwrap_or_default().to_string_lossy();

        if known_corpora.contains(&dir_name.as_ref()) {
            collect_pdfs_recursive(corpus_dir, corpus_dir, &dir_name, &mut targets)?;
        } else if !subdirs.is_empty() {
            for subdir in subdirs {
                let corpus_name = subdir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                collect_pdfs_recursive(&subdir, &subdir, &corpus_name, &mut targets)?;
            }
            if !root_pdfs.is_empty() {
                for path in root_pdfs {
                    let rel = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    targets.push(SweepTarget {
                        corpus: dir_name.to_string(),
                        relative_path: rel,
                        full_path: path,
                    });
                }
            }
        } else {
            collect_pdfs_recursive(corpus_dir, corpus_dir, &dir_name, &mut targets)?;
        }
    }

    targets.sort_by(|a, b| (&a.corpus, &a.relative_path).cmp(&(&b.corpus, &b.relative_path)));
    Ok(targets)
}

/// Sweeps a single PDF file through `host`.
pub fn sweep_file(
    host: &mut WorkerHost,
    corpus: &str,
    relative_path: &str,
    file_path: &Path,
    file_timeout: Duration,
) -> FileResult {
    let started = Instant::now();
    let metadata = match fs::metadata(file_path) {
        Ok(m) => m,
        Err(e) => {
            return FileResult {
                corpus: corpus.to_string(),
                relative_path: relative_path.to_string(),
                bytes: 0,
                pages: None,
                result: SweepResultKind::Unreadable,
                stage: SweepStage::ReadFile,
                page_index: None,
                error_code: Some(e.to_string()),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: None,
            };
        }
    };
    let file_bytes = metadata.len();

    // 1. Open document
    let open_res = host.open(file_path);
    let (doc, document) = match open_res {
        Ok((doc, WorkerResponse::Opened { document, .. })) => (doc, document),
        Ok((doc, other)) => {
            let _ = host.notify(&WorkerRequest::Close { doc });
            return FileResult {
                corpus: corpus.to_string(),
                relative_path: relative_path.to_string(),
                bytes: file_bytes,
                pages: None,
                result: SweepResultKind::ProtocolViolation,
                stage: SweepStage::Open,
                page_index: None,
                error_code: Some(format!("unexpected response to Open: {other:?}")),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: host.worker_peak_memory(),
            };
        }
        Err(err) => {
            let (res_kind, code) = match &err {
                HostError::Worker(werr) => {
                    (SweepResultKind::OpenFailed, format!("{:?}", werr.code))
                }
                HostError::Crashed => (SweepResultKind::WorkerCrashed, "Crashed".into()),
                HostError::Timeout => (SweepResultKind::WorkerTimeout, "Timeout".into()),
                HostError::ProtocolViolation(msg) => {
                    (SweepResultKind::ProtocolViolation, msg.clone())
                }
                HostError::Unreadable(io_err) => (SweepResultKind::Unreadable, io_err.to_string()),
                HostError::TooLarge => (SweepResultKind::OpenFailed, "TooLarge".into()),
                HostError::Spawn(e) => {
                    (SweepResultKind::WorkerCrashed, format!("SpawnFailed: {e}"))
                }
            };
            return FileResult {
                corpus: corpus.to_string(),
                relative_path: relative_path.to_string(),
                bytes: file_bytes,
                pages: None,
                result: res_kind,
                stage: SweepStage::Open,
                page_index: None,
                error_code: Some(code),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: host.worker_peak_memory(),
            };
        }
    };

    let total_pages = document.pages.len() as u32;

    // 2. Render each page and get its text
    for page_idx in 0..total_pages {
        // Enforce per-file timeout
        if started.elapsed() >= file_timeout {
            let _ = host.notify(&WorkerRequest::Close { doc });
            return FileResult {
                corpus: corpus.to_string(),
                relative_path: relative_path.to_string(),
                bytes: file_bytes,
                pages: Some(total_pages),
                result: SweepResultKind::FileTimeout,
                stage: SweepStage::FileTimeout,
                page_index: Some(page_idx),
                error_code: Some(format!(
                    "timed out after {}ms (completed {} of {} pages)",
                    started.elapsed().as_millis(),
                    page_idx,
                    total_pages
                )),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: host.worker_peak_memory(),
            };
        }

        // Render page
        let page = document.pages[page_idx as usize];
        let scale = match fit_scale(page, 1.0) {
            Ok(s) => s,
            Err(e) => {
                let _ = host.notify(&WorkerRequest::Close { doc });
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: SweepResultKind::RenderFailed,
                    stage: SweepStage::Render,
                    page_index: Some(page_idx),
                    error_code: Some(format!("fit_scale: {e}")),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                };
            }
        };

        let render_res = host.request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index: page_idx,
            scale,
            rotation: Rotation::None,
        });

        match render_res {
            Ok(WorkerResponse::Rendered { .. }) => {}
            Ok(other) => {
                let _ = host.notify(&WorkerRequest::Close { doc });
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: SweepResultKind::ProtocolViolation,
                    stage: SweepStage::Render,
                    page_index: Some(page_idx),
                    error_code: Some(format!("unexpected response to Render: {other:?}")),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                };
            }
            Err(err) => {
                let (res_kind, code) = match &err {
                    HostError::Worker(werr) => {
                        (SweepResultKind::RenderFailed, format!("{:?}", werr.code))
                    }
                    HostError::Crashed => (SweepResultKind::WorkerCrashed, "Crashed".into()),
                    HostError::Timeout => (SweepResultKind::WorkerTimeout, "Timeout".into()),
                    HostError::ProtocolViolation(msg) => {
                        (SweepResultKind::ProtocolViolation, msg.clone())
                    }
                    HostError::Unreadable(io_err) => {
                        (SweepResultKind::Unreadable, io_err.to_string())
                    }
                    HostError::TooLarge => (SweepResultKind::RenderFailed, "TooLarge".into()),
                    HostError::Spawn(e) => {
                        (SweepResultKind::WorkerCrashed, format!("SpawnFailed: {e}"))
                    }
                };
                if res_kind == SweepResultKind::RenderFailed {
                    let _ = host.notify(&WorkerRequest::Close { doc });
                }
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: res_kind,
                    stage: SweepStage::Render,
                    page_index: Some(page_idx),
                    error_code: Some(code),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                };
            }
        }

        // Check file timeout before GetPageText
        if started.elapsed() >= file_timeout {
            let _ = host.notify(&WorkerRequest::Close { doc });
            return FileResult {
                corpus: corpus.to_string(),
                relative_path: relative_path.to_string(),
                bytes: file_bytes,
                pages: Some(total_pages),
                result: SweepResultKind::FileTimeout,
                stage: SweepStage::FileTimeout,
                page_index: Some(page_idx),
                error_code: Some(format!(
                    "timed out after {}ms (rendered page {} of {})",
                    started.elapsed().as_millis(),
                    page_idx + 1,
                    total_pages
                )),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: host.worker_peak_memory(),
            };
        }

        // GetPageText
        let text_res = host.request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index: page_idx,
        });

        match text_res {
            Ok(WorkerResponse::PageText { .. }) => {}
            Ok(other) => {
                let _ = host.notify(&WorkerRequest::Close { doc });
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: SweepResultKind::ProtocolViolation,
                    stage: SweepStage::GetText,
                    page_index: Some(page_idx),
                    error_code: Some(format!("unexpected response to GetPageText: {other:?}")),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                };
            }
            Err(err) => {
                let (res_kind, code) = match &err {
                    HostError::Worker(werr) => {
                        (SweepResultKind::GetTextFailed, format!("{:?}", werr.code))
                    }
                    HostError::Crashed => (SweepResultKind::WorkerCrashed, "Crashed".into()),
                    HostError::Timeout => (SweepResultKind::WorkerTimeout, "Timeout".into()),
                    HostError::ProtocolViolation(msg) => {
                        (SweepResultKind::ProtocolViolation, msg.clone())
                    }
                    HostError::Unreadable(io_err) => {
                        (SweepResultKind::Unreadable, io_err.to_string())
                    }
                    HostError::TooLarge => (SweepResultKind::GetTextFailed, "TooLarge".into()),
                    HostError::Spawn(e) => {
                        (SweepResultKind::WorkerCrashed, format!("SpawnFailed: {e}"))
                    }
                };
                if res_kind == SweepResultKind::GetTextFailed {
                    let _ = host.notify(&WorkerRequest::Close { doc });
                }
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: res_kind,
                    stage: SweepStage::GetText,
                    page_index: Some(page_idx),
                    error_code: Some(code),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                };
            }
        }
    }

    // 3. Close document
    let _ = host.notify(&WorkerRequest::Close { doc });
    FileResult {
        corpus: corpus.to_string(),
        relative_path: relative_path.to_string(),
        bytes: file_bytes,
        pages: Some(total_pages),
        result: SweepResultKind::Ok,
        stage: SweepStage::Render,
        page_index: None,
        error_code: None,
        elapsed_ms: started.elapsed().as_millis() as u64,
        peak_memory_bytes: host.worker_peak_memory(),
    }
}

/// Runs the sweep over `corpus_dir` and writes results to `output_dir`.
pub fn run_sweep(
    worker_exe: &Path,
    corpus_dir: &Path,
    output_dir: &Path,
    file_timeout: Duration,
    expect_files: Option<usize>,
    specific_corpus: Option<&str>,
) -> Result<SweepSummary, String> {
    if !corpus_dir.is_dir() {
        return Err(format!(
            "corpus directory does not exist: {}",
            corpus_dir.display()
        ));
    }
    fs::create_dir_all(output_dir)
        .map_err(|e| format!("failed to create output dir {}: {e}", output_dir.display()))?;

    let targets = discover_files(corpus_dir, specific_corpus)
        .map_err(|e| format!("failed to discover files in {}: {e}", corpus_dir.display()))?;

    if targets.is_empty() {
        return Err(format!("no PDF files found in {}", corpus_dir.display()));
    }
    if let Some(expected) = expect_files
        && targets.len() != expected
    {
        return Err(format!(
            "expected {expected} files but discovered {} files",
            targets.len()
        ));
    }

    let results_path = output_dir.join("results.jsonl");
    let results_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&results_path)
        .map_err(|e| format!("failed to open results.jsonl: {e}"))?;
    let mut writer = BufWriter::new(results_file);

    println!(
        "corpus_sweep: starting sweep of {} files across {}",
        targets.len(),
        corpus_dir.display()
    );

    let sweep_started = Instant::now();
    let mut host = WorkerHost::new(worker_exe, HostConfig::default());
    let mut results = Vec::with_capacity(targets.len());

    for (i, target) in targets.iter().enumerate() {
        let res = sweep_file(
            &mut host,
            &target.corpus,
            &target.relative_path,
            &target.full_path,
            file_timeout,
        );

        let json_line = res.to_json_line();
        writeln!(writer, "{json_line}")
            .map_err(|e| format!("failed to write results.jsonl: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("failed to flush results.jsonl: {e}"))?;

        if (i + 1) % 50 == 0 || (i + 1) == targets.len() {
            println!(
                "[{}/{}] ({}) {} -> {} ({} ms)",
                i + 1,
                targets.len(),
                target.corpus,
                target.relative_path,
                res.result.as_str(),
                res.elapsed_ms
            );
        }

        results.push(res);
    }

    let total_elapsed = sweep_started.elapsed().as_millis() as u64;
    let summary = SweepSummary::from_results(&results, total_elapsed);

    // Write summary.md
    let md = summary.generate_markdown();
    fs::write(output_dir.join("summary.md"), &md)
        .map_err(|e| format!("failed to write summary.md: {e}"))?;

    // Write summary.json
    let json = summary.generate_json();
    fs::write(output_dir.join("summary.json"), &json)
        .map_err(|e| format!("failed to write summary.json: {e}"))?;

    println!(
        "corpus_sweep: completed {} files in {:.2}s. Output written to {}",
        summary.total_files,
        total_elapsed as f64 / 1000.0,
        output_dir.display()
    );

    Ok(summary)
}

/// Parses CLI arguments and runs `corpus_sweep`.
pub fn run_cli() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut positional = Vec::new();
    let mut file_seconds = 60u64;
    let mut expect_files = None;
    let mut worker_override = None;
    let mut specific_corpus = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--file-seconds" => {
                let val = args
                    .next()
                    .ok_or("--file-seconds requires a numeric argument")?;
                file_seconds = val
                    .parse()
                    .map_err(|_| "--file-seconds must be a valid number")?;
            }
            "--expect-files" => {
                let val = args
                    .next()
                    .ok_or("--expect-files requires a numeric argument")?;
                expect_files = Some(
                    val.parse()
                        .map_err(|_| "--expect-files must be a valid number")?,
                );
            }
            "--worker" => {
                let val = args.next().ok_or("--worker requires an executable path")?;
                worker_override = Some(PathBuf::from(val));
            }
            "--corpus" => {
                let val = args.next().ok_or("--corpus requires a corpus name")?;
                specific_corpus = Some(val);
            }
            "-h" | "--help" => {
                println!(
                    "Usage: corpus_sweep <corpus_dir> <output_dir> [--file-seconds 60] [--expect-files N] [--worker <path>] [--corpus <name>]"
                );
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}"));
            }
            _ => {
                positional.push(arg);
            }
        }
    }

    if positional.len() < 2 {
        return Err(
            "usage: corpus_sweep <corpus_dir> <output_dir> [--file-seconds 60] [--expect-files N]"
                .into(),
        );
    }

    let corpus_dir = PathBuf::from(&positional[0]);
    let output_dir = PathBuf::from(&positional[1]);
    let worker_exe = match worker_override {
        Some(p) => p,
        None => bundled_worker_path().map_err(|e| format!("cannot find worker exe: {e}"))?,
    };

    run_sweep(
        &worker_exe,
        &corpus_dir,
        &output_dir,
        Duration::from_secs(file_seconds),
        expect_files,
        specific_corpus.as_deref(),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_json() {
        assert_eq!(escape_json("hello \"world\""), "hello \\\"world\\\"");
        assert_eq!(escape_json("line1\nline2\r\t"), "line1\\nline2\\r\\t");
        assert_eq!(escape_json("path/to/file.pdf"), "path/to/file.pdf");
    }

    #[test]
    fn test_file_result_json_line() {
        let res = FileResult {
            corpus: "qpdf".into(),
            relative_path: "qtest/test.pdf".into(),
            bytes: 12345,
            pages: Some(3),
            result: SweepResultKind::Ok,
            stage: SweepStage::Render,
            page_index: None,
            error_code: None,
            elapsed_ms: 42,
            peak_memory_bytes: Some(10485760),
        };
        let line = res.to_json_line();
        assert!(line.contains("\"corpus\":\"qpdf\""));
        assert!(line.contains("\"relative_path\":\"qtest/test.pdf\""));
        assert!(line.contains("\"bytes\":12345"));
        assert!(line.contains("\"pages\":3"));
        assert!(line.contains("\"result\":\"ok\""));
        assert!(line.contains("\"stage\":\"render\""));
        assert!(line.contains("\"page_index\":null"));
        assert!(line.contains("\"error_code\":null"));
        assert!(line.contains("\"elapsed_ms\":42"));
        assert!(line.contains("\"peak_memory_bytes\":10485760"));
    }

    #[test]
    fn test_summary_ordering_and_markdown() {
        let results = vec![
            FileResult {
                corpus: "qpdf".into(),
                relative_path: "a.pdf".into(),
                bytes: 100,
                pages: Some(1),
                result: SweepResultKind::Ok,
                stage: SweepStage::Render,
                page_index: None,
                error_code: None,
                elapsed_ms: 10,
                peak_memory_bytes: None,
            },
            FileResult {
                corpus: "qpdf".into(),
                relative_path: "b.pdf".into(),
                bytes: 200,
                pages: None,
                result: SweepResultKind::OpenFailed,
                stage: SweepStage::Open,
                page_index: None,
                error_code: Some("Corrupt".into()),
                elapsed_ms: 5,
                peak_memory_bytes: None,
            },
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "crash.pdf".into(),
                bytes: 300,
                pages: Some(5),
                result: SweepResultKind::WorkerCrashed,
                stage: SweepStage::Render,
                page_index: Some(2),
                error_code: Some("Crashed".into()),
                elapsed_ms: 50,
                peak_memory_bytes: None,
            },
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "timeout.pdf".into(),
                bytes: 400,
                pages: Some(10),
                result: SweepResultKind::WorkerTimeout,
                stage: SweepStage::GetText,
                page_index: Some(4),
                error_code: Some("Timeout".into()),
                elapsed_ms: 30000,
                peak_memory_bytes: None,
            },
        ];

        let summary = SweepSummary::from_results(&results, 35000);
        assert_eq!(summary.total_files, 4);
        assert_eq!(summary.notable_issues.len(), 2);
        // Crash must come before timeout in notable issues:
        assert_eq!(
            summary.notable_issues[0].result,
            SweepResultKind::WorkerCrashed
        );
        assert_eq!(
            summary.notable_issues[1].result,
            SweepResultKind::WorkerTimeout
        );

        let md = summary.generate_markdown();
        assert!(md.contains("| `qpdf` | 2 | 1 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |"));
        assert!(md.contains("| `pdfjs` | 2 | 0 | 0 | 0 | 0 | 1 | 1 | 0 | 0 | 0 |"));
        assert!(md.contains("`Corrupt`"));
        assert!(md.contains("crash.pdf"));

        let json = summary.generate_json();
        assert!(json.contains("\"total_files\": 4"));
        assert!(json.contains("\"worker_crashed\": 1"));
        assert!(json.contains("\"worker_timeout\": 1"));
    }
}
