//! Public PDF corpus robustness scanner (Issue #439, B5-02).
//!
//! Sweeps untrusted PDF files through the sandboxed `pdf_worker` without parsing
//! them in the main process. Renders each page and extracts text, detecting crashes,
//! timeouts, protocol violations, or rendering/extraction failures.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ipc_contract::raster::fit_scale;
use ipc_contract::types::Rotation;
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use sha2::{Digest, Sha256};

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

    /// Whether this result category must have its file name and details withheld in public output.
    pub fn is_withheld(&self) -> bool {
        matches!(
            self,
            Self::WorkerCrashed | Self::ProtocolViolation | Self::WorkerTimeout | Self::FileTimeout
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

/// A record of hashes for one page of a scanned PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageHashRecord {
    pub corpus: String,
    pub relative_path: String,
    pub page_index: u32,
    pub render_hash: String,
    pub text_hash: String,
    pub dhash: String,
}

impl PageHashRecord {
    pub fn to_json_line(&self) -> String {
        format!(
            "{{\"corpus\":\"{}\",\"relative_path\":\"{}\",\"page\":{},\"render_hash\":\"{}\",\"text_hash\":\"{}\",\"dhash\":\"{}\"}}",
            escape_json(&self.corpus),
            escape_json(&self.relative_path),
            self.page_index,
            self.render_hash,
            self.text_hash,
            self.dhash,
        )
    }

    pub fn from_json_line(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        fn extract_str(json: &str, key: &str) -> Option<String> {
            let key_pattern = format!("\"{key}\":\"");
            let start = json.find(&key_pattern)? + key_pattern.len();
            let rest = &json[start..];
            let mut val = String::new();
            let mut chars = rest.chars();
            while let Some(c) = chars.next() {
                if c == '"' {
                    return Some(val);
                } else if c == '\\' {
                    if let Some(escaped) = chars.next() {
                        match escaped {
                            '"' => val.push('"'),
                            '\\' => val.push('\\'),
                            'n' => val.push('\n'),
                            'r' => val.push('\r'),
                            't' => val.push('\t'),
                            other => {
                                val.push('\\');
                                val.push(other);
                            }
                        }
                    }
                } else {
                    val.push(c);
                }
            }
            None
        }

        fn extract_u32(json: &str, key: &str) -> Option<u32> {
            let key_pattern = format!("\"{key}\":");
            let start = json.find(&key_pattern)? + key_pattern.len();
            let rest = json[start..].trim_start();
            let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            num_str.parse().ok()
        }

        let corpus = extract_str(line, "corpus")?;
        let relative_path = extract_str(line, "relative_path")?;
        let page_index = extract_u32(line, "page")?;
        let render_hash = extract_str(line, "render_hash")?;
        let text_hash = extract_str(line, "text_hash")?;
        let dhash = extract_str(line, "dhash")?;

        Some(Self {
            corpus,
            relative_path,
            page_index,
            render_hash,
            text_hash,
            dhash,
        })
    }
}

fn to_hex_str(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Computes SHA-256 hash of rendered raster (width, height, and RGBA pixels).
pub fn compute_render_hash(raster: &ipc_contract::worker::Raster) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raster.width.to_le_bytes());
    hasher.update(raster.height.to_le_bytes());
    hasher.update(&raster.pixels);
    to_hex_str(&hasher.finalize())
}

/// Computes SHA-256 hash of extracted plain text from `PageText` (lines joined by newline).
pub fn compute_text_hash(page_text: &ipc_contract::types::PageText) -> String {
    let mut hasher = Sha256::new();
    for (i, line) in page_text.lines.iter().enumerate() {
        if i > 0 {
            hasher.update(b"\n");
        }
        hasher.update(line.text.as_bytes());
    }
    to_hex_str(&hasher.finalize())
}

/// Computes a 64-bit perceptual difference hash (dHash) from raster RGBA pixels.
/// Downsamples the image to a 16x16 grayscale grid, averages 2x2 cells to 8x8,
/// and computes circular adjacent column differences to generate 64 bits.
pub fn compute_dhash(raster: &ipc_contract::worker::Raster) -> u64 {
    let width = raster.width as usize;
    let height = raster.height as usize;
    if width == 0 || height == 0 || raster.pixels.len() < width * height * 4 {
        return 0;
    }

    // 1. Downsample to 16x16 grayscale by box averaging
    let mut gray_16x16 = [0u8; 16 * 16];
    for gy in 0..16 {
        for gx in 0..16 {
            let x_start = gx * width / 16;
            let x_end = ((gx + 1) * width / 16).max(x_start + 1).min(width);
            let y_start = gy * height / 16;
            let y_end = ((gy + 1) * height / 16).max(y_start + 1).min(height);

            let mut sum_lum = 0u64;
            let mut count = 0u64;
            for y in y_start..y_end {
                for x in x_start..x_end {
                    let offset = (y * width + x) * 4;
                    let r = raster.pixels[offset] as u64;
                    let g = raster.pixels[offset + 1] as u64;
                    let b = raster.pixels[offset + 2] as u64;
                    let lum = (r * 299 + g * 587 + b * 114) / 1000;
                    sum_lum += lum;
                    count += 1;
                }
            }
            gray_16x16[gy * 16 + gx] = sum_lum.checked_div(count).map(|v| v as u8).unwrap_or(0);
        }
    }

    // 2. Average 2x2 blocks into 8x8 grid
    let mut grid8x8 = [[0u8; 8]; 8];
    for by in 0..8 {
        for bx in 0..8 {
            let p00 = gray_16x16[(by * 2) * 16 + (bx * 2)] as u32;
            let p01 = gray_16x16[(by * 2) * 16 + (bx * 2 + 1)] as u32;
            let p10 = gray_16x16[(by * 2 + 1) * 16 + (bx * 2)] as u32;
            let p11 = gray_16x16[(by * 2 + 1) * 16 + (bx * 2 + 1)] as u32;
            grid8x8[by][bx] = ((p00 + p01 + p10 + p11) / 4) as u8;
        }
    }

    // 3. Circular difference comparison to produce 64-bit dHash
    let mut hash = 0u64;
    for (r, row) in grid8x8.iter().enumerate() {
        for c in 0..8 {
            let next_c = (c + 1) % 8;
            if row[c] > row[next_c] {
                hash |= 1u64 << (r * 8 + c);
            }
        }
    }

    hash
}

/// Returns the Hamming distance between two 64-bit perceptual hashes.
pub fn hamming_distance(h1: u64, h2: u64) -> u32 {
    (h1 ^ h2).count_ones()
}

/// Difference kind: text content or rendered raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Text,
    Render,
}

impl DiffKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Render => "render",
        }
    }
}

impl std::str::FromStr for DiffKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "render" => Ok(Self::Render),
            _ => Err(()),
        }
    }
}

/// An approved difference rule from `approved-differences.tsv`.
#[derive(Debug, Clone)]
pub struct ApprovedEntry {
    pub corpus: String,
    pub relative_path: String, // "*" or specific path
    pub page: Option<u32>,     // None if "*", Some(1-based page number) if numeric
    pub kind: DiffKind,
    pub reason: String,
    pub pr: String,
    pub line_number: usize,
    pub match_count: usize,
}

/// A detected difference for a specific page.
#[derive(Debug, Clone)]
pub struct PageDifference {
    pub corpus: String,
    pub relative_path: String,
    pub page_number: u32, // 1-based page number
    pub kind: DiffKind,
    pub dhash_distance: Option<u32>,
    pub baseline_hash: Option<String>,
    pub current_hash: Option<String>,
    pub approved_by: Option<(String, String)>, // (reason, pr)
}

/// Aggregated report of differential testing against a baseline.
#[derive(Debug, Default, Clone)]
pub struct DifferentialReport {
    pub baseline_page_count: usize,
    pub current_page_count: usize,
    pub changed_pages_count: usize,
    pub approved_count: usize,
    pub unapproved_count: usize,
    pub withheld_files_count: usize,
    pub differences: Vec<PageDifference>,
    pub unused_approvals: Vec<ApprovedEntry>,
    pub new_files: Vec<String>,
    pub baseline_available: bool,
}

impl DifferentialReport {
    pub fn generate_markdown(&self) -> String {
        let mut md = String::new();
        md.push_str("## 語料差異比對 (Differential Testing Against Baseline)\n\n");
        if !self.baseline_available {
            md.push_str("沒有基準，這次只記錄。\n\n");
            return md;
        }

        md.push_str(&format!(
            "- **基準總頁數**：{} 頁\n- **本次總頁數**：{} 頁\n- **改變的頁數**：{} 頁\n- **核准的差異**：{} 項\n- **未核准的差異**：{} 項\n- **未使用的核准規則**：{} 列\n\n",
            self.baseline_page_count,
            self.current_page_count,
            self.changed_pages_count,
            self.approved_count,
            self.unapproved_count,
            self.unused_approvals.len(),
        ));

        if !self.new_files.is_empty() {
            md.push_str(&format!(
                "- **基準中未包含的新檔案**：{} 個（僅記錄，不列入失敗）\n\n",
                self.new_files.len()
            ));
        }

        if self.withheld_files_count > 0 {
            md.push_str(&format!(
                "- **異常而隱藏名稱的檔案**：{} 個檔案這次異常，名稱不公開（不列出路徑、頁碼或雜湊，仍視為未核准差異）\n\n",
                self.withheld_files_count
            ));
        }

        if self.differences.is_empty() && self.withheld_files_count == 0 {
            md.push_str("未發現任何與基準不同的頁面。\n\n");
        } else if !self.differences.is_empty() {
            md.push_str("### 差異清單 (Differences)\n\n");
            md.push_str("| 語料 | 檔案路徑 | 頁碼 | 種類 | 感知雜湊距離 | 狀態 | 理由 / PR |\n");
            md.push_str("|---|---|---|---|---|---|---|\n");
            for diff in &self.differences {
                let dist_str = diff
                    .dhash_distance
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "-".into());
                let (status_str, reason_col) = match &diff.approved_by {
                    Some((reason, pr)) => ("核准 (approved)", format!("{reason} ({pr})")),
                    None => ("未核准 (UNAPPROVED)", "-".into()),
                };
                md.push_str(&format!(
                    "| `{}` | `{}` | 第 {} 頁 | `{}` | {} | {} | {} |\n",
                    diff.corpus,
                    diff.relative_path,
                    diff.page_number,
                    diff.kind.as_str(),
                    dist_str,
                    status_str,
                    reason_col
                ));
            }
            md.push('\n');
        }

        if !self.unused_approvals.is_empty() {
            md.push_str("### 未使用的核准規則 (Unused Approval Rules)\n\n");
            md.push_str("> 警告：以下核准規則未對應到任何實際差異，基準更新後應予以移除。\n\n");
            md.push_str("| 語料 | 檔案路徑 | 頁碼 | 種類 | 理由 | 核准 PR | 規則行號 |\n");
            md.push_str("|---|---|---|---|---|---|---|\n");
            for u in &self.unused_approvals {
                let page_str = u
                    .page
                    .map(|p| format!("第 {} 頁", p))
                    .unwrap_or_else(|| "*".into());
                md.push_str(&format!(
                    "| `{}` | `{}` | {} | `{}` | {} | {} | 第 {} 行 |\n",
                    u.corpus,
                    u.relative_path,
                    page_str,
                    u.kind.as_str(),
                    u.reason,
                    u.pr,
                    u.line_number
                ));
            }
            md.push('\n');
        }

        md
    }
}

/// Reads approved differences from a TSV file.
pub fn load_approved_tsv(path: &Path) -> Result<Vec<ApprovedEntry>, String> {
    let content = fs::read_to_string(path).map_err(|e| {
        format!(
            "failed to read approved differences file {}: {e}",
            path.display()
        )
    })?;
    let mut entries = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = trimmed.split('\t').map(|s| s.trim()).collect();
        if parts.is_empty() {
            continue;
        }
        if parts[0] == "corpus" || parts[0] == "語料" {
            continue;
        }

        if parts.len() < 6 {
            return Err(format!(
                "{}:{line_num}: expected 6 tab-separated columns (corpus, relative_path, page, kind, reason, pr), got {}",
                path.display(),
                parts.len()
            ));
        }

        let corpus = parts[0].to_string();
        let relative_path = parts[1].replace('\\', "/");
        let page_str = parts[2];
        let kind_str = parts[3];
        let reason = parts[4].to_string();
        let pr = parts[5].to_string();

        let page = if page_str == "*" {
            None
        } else {
            match page_str.parse::<u32>() {
                Ok(p) => Some(p),
                Err(_) => {
                    return Err(format!(
                        "{}:{line_num}: invalid page number '{page_str}', expected 1-based number or '*'",
                        path.display()
                    ));
                }
            }
        };

        let kind = kind_str.parse::<DiffKind>().map_err(|_| {
            format!(
                "{}:{line_num}: invalid diff kind '{kind_str}', expected 'text' or 'render'",
                path.display()
            )
        })?;

        entries.push(ApprovedEntry {
            corpus,
            relative_path,
            page,
            kind,
            reason,
            pr,
            line_number: line_num,
            match_count: 0,
        });
    }

    Ok(entries)
}

/// Compares current page hashes against baseline page hashes and checks against approvals.
///
/// Files in `withheld_files` (pairs of `(corpus, relative_path)`) that appear in `baseline_records`
/// are counted as unapproved differences via `withheld_files_count` without recording their paths,
/// page numbers, or hashes in `differences`.
pub fn compare_page_hashes(
    baseline_records: &[PageHashRecord],
    current_records: &[PageHashRecord],
    mut approvals: Vec<ApprovedEntry>,
    withheld_files: &BTreeSet<(String, String)>,
) -> DifferentialReport {
    let mut baseline_by_file: BTreeMap<(String, String), BTreeMap<u32, PageHashRecord>> =
        BTreeMap::new();
    for rec in baseline_records {
        baseline_by_file
            .entry((rec.corpus.clone(), rec.relative_path.clone()))
            .or_default()
            .insert(rec.page_index, rec.clone());
    }

    let mut current_by_file: BTreeMap<(String, String), BTreeMap<u32, PageHashRecord>> =
        BTreeMap::new();
    let mut current_page_count = 0usize;
    for rec in current_records {
        let key = (rec.corpus.clone(), rec.relative_path.clone());
        if withheld_files.contains(&key) {
            continue;
        }
        current_page_count += 1;
        current_by_file
            .entry(key)
            .or_default()
            .insert(rec.page_index, rec.clone());
    }

    let mut differences = Vec::new();
    let mut new_files = Vec::new();
    let mut withheld_files_count = 0usize;

    // Check files in current not in baseline
    for (corpus, rel_path) in current_by_file.keys() {
        let key = (corpus.clone(), rel_path.clone());
        if !baseline_by_file.contains_key(&key) && !withheld_files.contains(&key) {
            new_files.push(format!("{corpus}/{rel_path}"));
        }
    }

    // For files in baseline:
    for ((corpus, rel_path), b_pages) in &baseline_by_file {
        if withheld_files.contains(&(corpus.clone(), rel_path.clone())) {
            withheld_files_count += 1;
            continue;
        }
        match current_by_file.get(&(corpus.clone(), rel_path.clone())) {
            None => {
                // File completely missing in current run (failed to open / crashed / missing)
                for (p_idx, b_rec) in b_pages {
                    let page_num = p_idx + 1;
                    differences.push(PageDifference {
                        corpus: corpus.clone(),
                        relative_path: rel_path.clone(),
                        page_number: page_num,
                        kind: DiffKind::Text,
                        dhash_distance: None,
                        baseline_hash: Some(b_rec.text_hash.clone()),
                        current_hash: None,
                        approved_by: None,
                    });
                    differences.push(PageDifference {
                        corpus: corpus.clone(),
                        relative_path: rel_path.clone(),
                        page_number: page_num,
                        kind: DiffKind::Render,
                        dhash_distance: None,
                        baseline_hash: Some(b_rec.render_hash.clone()),
                        current_hash: None,
                        approved_by: None,
                    });
                }
            }
            Some(c_pages) => {
                // Check all pages in baseline
                for (p_idx, b_rec) in b_pages {
                    let page_num = p_idx + 1;
                    match c_pages.get(p_idx) {
                        None => {
                            // Page missing in current run (fewer pages)
                            differences.push(PageDifference {
                                corpus: corpus.clone(),
                                relative_path: rel_path.clone(),
                                page_number: page_num,
                                kind: DiffKind::Text,
                                dhash_distance: None,
                                baseline_hash: Some(b_rec.text_hash.clone()),
                                current_hash: None,
                                approved_by: None,
                            });
                            differences.push(PageDifference {
                                corpus: corpus.clone(),
                                relative_path: rel_path.clone(),
                                page_number: page_num,
                                kind: DiffKind::Render,
                                dhash_distance: None,
                                baseline_hash: Some(b_rec.render_hash.clone()),
                                current_hash: None,
                                approved_by: None,
                            });
                        }
                        Some(c_rec) => {
                            // Check text
                            if b_rec.text_hash != c_rec.text_hash {
                                differences.push(PageDifference {
                                    corpus: corpus.clone(),
                                    relative_path: rel_path.clone(),
                                    page_number: page_num,
                                    kind: DiffKind::Text,
                                    dhash_distance: None,
                                    baseline_hash: Some(b_rec.text_hash.clone()),
                                    current_hash: Some(c_rec.text_hash.clone()),
                                    approved_by: None,
                                });
                            }
                            // Check render
                            if b_rec.render_hash != c_rec.render_hash {
                                let d1 = u64::from_str_radix(&b_rec.dhash, 16).unwrap_or(0);
                                let d2 = u64::from_str_radix(&c_rec.dhash, 16).unwrap_or(0);
                                let dist = hamming_distance(d1, d2);
                                differences.push(PageDifference {
                                    corpus: corpus.clone(),
                                    relative_path: rel_path.clone(),
                                    page_number: page_num,
                                    kind: DiffKind::Render,
                                    dhash_distance: Some(dist),
                                    baseline_hash: Some(b_rec.render_hash.clone()),
                                    current_hash: Some(c_rec.render_hash.clone()),
                                    approved_by: None,
                                });
                            }
                        }
                    }
                }

                // Check if current has extra pages not in baseline for this file
                for (p_idx, c_rec) in c_pages {
                    if !b_pages.contains_key(p_idx) {
                        let page_num = p_idx + 1;
                        differences.push(PageDifference {
                            corpus: corpus.clone(),
                            relative_path: rel_path.clone(),
                            page_number: page_num,
                            kind: DiffKind::Text,
                            dhash_distance: None,
                            baseline_hash: None,
                            current_hash: Some(c_rec.text_hash.clone()),
                            approved_by: None,
                        });
                        differences.push(PageDifference {
                            corpus: corpus.clone(),
                            relative_path: rel_path.clone(),
                            page_number: page_num,
                            kind: DiffKind::Render,
                            dhash_distance: None,
                            baseline_hash: None,
                            current_hash: Some(c_rec.render_hash.clone()),
                            approved_by: None,
                        });
                    }
                }
            }
        }
    }

    // Sort differences deterministically: corpus -> path -> page -> kind
    differences.sort_by(|a, b| {
        a.corpus
            .cmp(&b.corpus)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
            .then_with(|| a.page_number.cmp(&b.page_number))
            .then_with(|| a.kind.as_str().cmp(b.kind.as_str()))
    });

    // Match against approved entries
    let mut approved_count = 0;
    let mut unapproved_count = 0;

    for diff in &mut differences {
        let mut matched = false;
        for entry in &mut approvals {
            let corpus_match = entry.corpus == diff.corpus;
            let path_match =
                entry.relative_path == "*" || entry.relative_path == diff.relative_path;
            let page_match = entry.page.is_none() || entry.page == Some(diff.page_number);
            let kind_match = entry.kind == diff.kind;

            if corpus_match && path_match && page_match && kind_match {
                entry.match_count += 1;
                if !matched {
                    diff.approved_by = Some((entry.reason.clone(), entry.pr.clone()));
                    matched = true;
                }
            }
        }
        if matched {
            approved_count += 1;
        } else {
            unapproved_count += 1;
        }
    }

    let mut unused_approvals = Vec::new();
    for entry in approvals {
        if entry.match_count == 0 {
            unused_approvals.push(entry);
        }
    }

    let mut unique_changed_pages = BTreeSet::new();
    for diff in &differences {
        unique_changed_pages.insert((
            diff.corpus.clone(),
            diff.relative_path.clone(),
            diff.page_number,
        ));
    }

    unapproved_count += withheld_files_count;

    DifferentialReport {
        baseline_page_count: baseline_records.len(),
        current_page_count,
        changed_pages_count: unique_changed_pages.len(),
        approved_count,
        unapproved_count,
        withheld_files_count,
        differences,
        unused_approvals,
        new_files,
        baseline_available: true,
    }
}

/// Collects current page hashes and withheld file identifiers from scan results
/// and compares them against baseline records.
pub fn compare_results_against_baseline(
    baseline_records: &[PageHashRecord],
    results: &[FileResult],
    approvals: Vec<ApprovedEntry>,
    withhold_names: bool,
) -> DifferentialReport {
    let mut current_records = Vec::new();
    let mut withheld_files = BTreeSet::new();
    for r in results {
        if withhold_names && r.result.is_withheld() {
            withheld_files.insert((r.corpus.clone(), r.relative_path.clone()));
        } else {
            current_records.extend(r.page_hashes.clone());
        }
    }
    compare_page_hashes(
        baseline_records,
        &current_records,
        approvals,
        &withheld_files,
    )
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
    pub page_hashes: Vec<PageHashRecord>,
}

impl FileResult {
    pub fn to_json_line(&self, withhold_names: bool) -> String {
        let (rel_path, bytes_str, pages_str, page_idx_str, err_str, elapsed_str, peak_mem_str) =
            if withhold_names && self.result.is_withheld() {
                (
                    "(withheld)".to_string(),
                    "null".to_string(),
                    "null".to_string(),
                    "null".to_string(),
                    "null".to_string(),
                    "null".to_string(),
                    "null".to_string(),
                )
            } else {
                (
                    escape_json(&self.relative_path),
                    self.bytes.to_string(),
                    self.pages
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "null".into()),
                    self.page_index
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "null".into()),
                    self.error_code
                        .as_ref()
                        .map(|c| format!("\"{}\"", escape_json(c)))
                        .unwrap_or_else(|| "null".into()),
                    self.elapsed_ms.to_string(),
                    self.peak_memory_bytes
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "null".into()),
                )
            };

        format!(
            "{{\"corpus\":\"{}\",\"relative_path\":\"{}\",\"bytes\":{},\"pages\":{},\"result\":\"{}\",\"stage\":\"{}\",\"page_index\":{},\"error_code\":{},\"elapsed_ms\":{},\"peak_memory_bytes\":{}}}",
            escape_json(&self.corpus),
            rel_path,
            bytes_str,
            pages_str,
            self.result.as_str(),
            self.stage.as_str(),
            page_idx_str,
            err_str,
            elapsed_str,
            peak_mem_str,
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
    pub fn generate_markdown(
        &self,
        withhold_names: bool,
        diff_report: Option<&DifferentialReport>,
    ) -> String {
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
                let (path_str, page_str, err_col, elapsed_str) =
                    if withhold_names && item.result.is_withheld() {
                        (
                            "(withheld)".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                            "-".to_string(),
                        )
                    } else {
                        let page_str = item
                            .page_index
                            .map(|p| format!("第 {} 頁", p + 1))
                            .unwrap_or_else(|| "-".into());
                        let err_str = item
                            .error_code
                            .as_deref()
                            .map(|c| format!("`{}`", c.replace('\n', " ")))
                            .unwrap_or_else(|| "-".into());
                        (
                            item.relative_path.clone(),
                            page_str,
                            err_str,
                            item.elapsed_ms.to_string(),
                        )
                    };
                md.push_str(&format!(
                    "| `{}` | `{}` | {} | {} | {} | {} | {} |\n",
                    item.corpus,
                    path_str,
                    item.result.display_name(),
                    item.stage.display_name(),
                    page_str,
                    err_col,
                    elapsed_str
                ));
            }
        }

        if let Some(diff) = diff_report {
            md.push('\n');
            md.push_str(&diff.generate_markdown());
        }

        md
    }

    /// Generates structured JSON summary (`summary.json`).
    pub fn generate_json(&self, diff_report: Option<&DifferentialReport>) -> String {
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
        out.push_str("  }");

        if let Some(diff) = diff_report {
            out.push_str(",\n  \"differential\": {\n");
            out.push_str(&format!(
                "    \"baseline_available\": {},\n",
                diff.baseline_available
            ));
            out.push_str(&format!(
                "    \"baseline_pages\": {},\n",
                diff.baseline_page_count
            ));
            out.push_str(&format!(
                "    \"current_pages\": {},\n",
                diff.current_page_count
            ));
            out.push_str(&format!(
                "    \"changed_pages\": {},\n",
                diff.changed_pages_count
            ));
            out.push_str(&format!("    \"approved\": {},\n", diff.approved_count));
            out.push_str(&format!("    \"unapproved\": {},\n", diff.unapproved_count));
            out.push_str(&format!(
                "    \"withheld_files\": {},\n",
                diff.withheld_files_count
            ));
            out.push_str(&format!(
                "    \"unused_approvals\": {}\n",
                diff.unused_approvals.len()
            ));
            out.push_str("  }\n");
        } else {
            out.push('\n');
        }
        out.push_str("}\n");

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
                error_code: Some(format!("{:?}", e.kind())),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: None,
                page_hashes: Vec::new(),
            };
        }
    };
    let file_bytes = metadata.len();

    // 1. Open document
    let open_res = host.open(file_path);
    let (doc, document) = match open_res {
        Ok((doc, WorkerResponse::Opened { document, .. })) => (doc, document),
        Ok((doc, _other)) => {
            let _ = host.notify(&WorkerRequest::Close { doc });
            return FileResult {
                corpus: corpus.to_string(),
                relative_path: relative_path.to_string(),
                bytes: file_bytes,
                pages: None,
                result: SweepResultKind::ProtocolViolation,
                stage: SweepStage::Open,
                page_index: None,
                error_code: Some("UnexpectedResponse".into()),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: host.worker_peak_memory(),
                page_hashes: Vec::new(),
            };
        }
        Err(err) => {
            let (res_kind, code) = match &err {
                HostError::Worker(werr) => {
                    (SweepResultKind::OpenFailed, format!("{:?}", werr.code))
                }
                HostError::Crashed => (SweepResultKind::WorkerCrashed, "Crashed".into()),
                HostError::Timeout => (SweepResultKind::WorkerTimeout, "Timeout".into()),
                HostError::ProtocolViolation(_) => (
                    SweepResultKind::ProtocolViolation,
                    "ProtocolViolation".into(),
                ),
                HostError::Unreadable(io_err) => {
                    (SweepResultKind::Unreadable, format!("{:?}", io_err.kind()))
                }
                HostError::TooLarge => (SweepResultKind::OpenFailed, "TooLarge".into()),
                HostError::Spawn(e) => (SweepResultKind::WorkerCrashed, format!("{:?}", e.kind())),
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
                page_hashes: Vec::new(),
            };
        }
    };

    let total_pages = document.pages.len() as u32;
    let mut page_hashes = Vec::with_capacity(total_pages as usize);

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
                error_code: Some("FileTimeout".into()),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: host.worker_peak_memory(),
                page_hashes: Vec::new(),
            };
        }

        // Render page
        let page = document.pages[page_idx as usize];
        let scale = match fit_scale(page, 1.0) {
            Ok(s) => s,
            Err(_e) => {
                let _ = host.notify(&WorkerRequest::Close { doc });
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: SweepResultKind::RenderFailed,
                    stage: SweepStage::Render,
                    page_index: Some(page_idx),
                    error_code: Some("FitScaleFailed".into()),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                    page_hashes: Vec::new(),
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

        let (render_hash, dhash) = match render_res {
            Ok(WorkerResponse::Rendered { raster, .. }) => {
                let r_hash = compute_render_hash(&raster);
                let d_hash = compute_dhash(&raster);
                (r_hash, format!("{d_hash:016x}"))
            }
            Ok(_other) => {
                let _ = host.notify(&WorkerRequest::Close { doc });
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: SweepResultKind::ProtocolViolation,
                    stage: SweepStage::Render,
                    page_index: Some(page_idx),
                    error_code: Some("UnexpectedResponse".into()),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                    page_hashes: Vec::new(),
                };
            }
            Err(err) => {
                let (res_kind, code) = match &err {
                    HostError::Worker(werr) => {
                        (SweepResultKind::RenderFailed, format!("{:?}", werr.code))
                    }
                    HostError::Crashed => (SweepResultKind::WorkerCrashed, "Crashed".into()),
                    HostError::Timeout => (SweepResultKind::WorkerTimeout, "Timeout".into()),
                    HostError::ProtocolViolation(_) => (
                        SweepResultKind::ProtocolViolation,
                        "ProtocolViolation".into(),
                    ),
                    HostError::Unreadable(io_err) => {
                        (SweepResultKind::Unreadable, format!("{:?}", io_err.kind()))
                    }
                    HostError::TooLarge => (SweepResultKind::RenderFailed, "TooLarge".into()),
                    HostError::Spawn(e) => {
                        (SweepResultKind::WorkerCrashed, format!("{:?}", e.kind()))
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
                    page_hashes: Vec::new(),
                };
            }
        };

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
                error_code: Some("FileTimeout".into()),
                elapsed_ms: started.elapsed().as_millis() as u64,
                peak_memory_bytes: host.worker_peak_memory(),
                page_hashes: Vec::new(),
            };
        }

        // GetPageText
        let text_res = host.request(|request| WorkerRequest::GetPageText {
            request,
            doc,
            page_index: page_idx,
        });

        match text_res {
            Ok(WorkerResponse::PageText { text, .. }) => {
                let text_hash = compute_text_hash(&text);
                page_hashes.push(PageHashRecord {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    page_index: page_idx,
                    render_hash,
                    text_hash,
                    dhash,
                });
            }
            Ok(_other) => {
                let _ = host.notify(&WorkerRequest::Close { doc });
                return FileResult {
                    corpus: corpus.to_string(),
                    relative_path: relative_path.to_string(),
                    bytes: file_bytes,
                    pages: Some(total_pages),
                    result: SweepResultKind::ProtocolViolation,
                    stage: SweepStage::GetText,
                    page_index: Some(page_idx),
                    error_code: Some("UnexpectedResponse".into()),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    peak_memory_bytes: host.worker_peak_memory(),
                    page_hashes: Vec::new(),
                };
            }
            Err(err) => {
                let (res_kind, code) = match &err {
                    HostError::Worker(werr) => {
                        (SweepResultKind::GetTextFailed, format!("{:?}", werr.code))
                    }
                    HostError::Crashed => (SweepResultKind::WorkerCrashed, "Crashed".into()),
                    HostError::Timeout => (SweepResultKind::WorkerTimeout, "Timeout".into()),
                    HostError::ProtocolViolation(_) => (
                        SweepResultKind::ProtocolViolation,
                        "ProtocolViolation".into(),
                    ),
                    HostError::Unreadable(io_err) => {
                        (SweepResultKind::Unreadable, format!("{:?}", io_err.kind()))
                    }
                    HostError::TooLarge => (SweepResultKind::GetTextFailed, "TooLarge".into()),
                    HostError::Spawn(e) => {
                        (SweepResultKind::WorkerCrashed, format!("{:?}", e.kind()))
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
                    page_hashes: Vec::new(),
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
        page_hashes,
    }
}

/// Runs the sweep over `corpus_dir` and writes results to `output_dir`.
#[allow(clippy::too_many_arguments)]
pub fn run_sweep(
    worker_exe: &Path,
    corpus_dir: &Path,
    output_dir: &Path,
    file_timeout: Duration,
    expect_files: Option<usize>,
    specific_corpus: Option<&str>,
    withhold_names: bool,
    baseline_dir: Option<&Path>,
    approved_file: Option<&Path>,
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

    let hashes_path = output_dir.join("page-hashes.jsonl");
    let hashes_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&hashes_path)
        .map_err(|e| format!("failed to open page-hashes.jsonl: {e}"))?;
    let mut hashes_writer = BufWriter::new(hashes_file);

    println!(
        "corpus_sweep: starting sweep of {} files across {}",
        targets.len(),
        corpus_dir.display()
    );

    let sweep_started = Instant::now();
    let mut results = Vec::with_capacity(targets.len());

    for (i, target) in targets.iter().enumerate() {
        let mut host = WorkerHost::new(worker_exe, HostConfig::default());
        let res = sweep_file(
            &mut host,
            &target.corpus,
            &target.relative_path,
            &target.full_path,
            file_timeout,
        );

        let json_line = res.to_json_line(withhold_names);
        writeln!(writer, "{json_line}")
            .map_err(|e| format!("failed to write results.jsonl: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("failed to flush results.jsonl: {e}"))?;

        if !(withhold_names && res.result.is_withheld()) {
            for ph in &res.page_hashes {
                writeln!(hashes_writer, "{}", ph.to_json_line())
                    .map_err(|e| format!("failed to write page-hashes.jsonl: {e}"))?;
            }
            hashes_writer
                .flush()
                .map_err(|e| format!("failed to flush page-hashes.jsonl: {e}"))?;
        }

        if (i + 1) % 50 == 0 || (i + 1) == targets.len() {
            let display_path = if withhold_names && res.result.is_withheld() {
                "(withheld)"
            } else {
                target.relative_path.as_str()
            };
            println!(
                "[{}/{}] ({}) {} -> {} ({} ms)",
                i + 1,
                targets.len(),
                target.corpus,
                display_path,
                res.result.as_str(),
                res.elapsed_ms
            );
        }

        results.push(res);
    }

    let total_elapsed = sweep_started.elapsed().as_millis() as u64;
    let summary = SweepSummary::from_results(&results, total_elapsed);

    // Differential testing if baseline is provided
    let mut diff_report = None;
    if let Some(b_dir) = baseline_dir {
        let b_hashes_path = b_dir.join("page-hashes.jsonl");
        if !b_hashes_path.is_file() {
            println!("沒有基準，這次只記錄");
            diff_report = Some(DifferentialReport {
                baseline_available: false,
                ..Default::default()
            });
        } else {
            let b_content = fs::read_to_string(&b_hashes_path)
                .map_err(|e| format!("failed to read baseline page-hashes.jsonl: {e}"))?;
            let mut baseline_records = Vec::new();
            for line in b_content.lines() {
                if let Some(rec) = PageHashRecord::from_json_line(line) {
                    baseline_records.push(rec);
                }
            }
            if baseline_records.is_empty() {
                println!("沒有基準，這次只記錄");
                diff_report = Some(DifferentialReport {
                    baseline_available: false,
                    ..Default::default()
                });
            } else {
                let approvals = if let Some(app_path) = approved_file {
                    load_approved_tsv(app_path)?
                } else {
                    Vec::new()
                };
                let report = compare_results_against_baseline(
                    &baseline_records,
                    &results,
                    approvals,
                    withhold_names,
                );
                diff_report = Some(report);
            }
        }
    }

    // Write summary.md
    let md = summary.generate_markdown(withhold_names, diff_report.as_ref());
    fs::write(output_dir.join("summary.md"), &md)
        .map_err(|e| format!("failed to write summary.md: {e}"))?;

    // Write summary.json
    let json = summary.generate_json(diff_report.as_ref());
    fs::write(output_dir.join("summary.json"), &json)
        .map_err(|e| format!("failed to write summary.json: {e}"))?;

    println!(
        "corpus_sweep: completed {} files in {:.2}s. Output written to {}",
        summary.total_files,
        total_elapsed as f64 / 1000.0,
        output_dir.display()
    );

    if let Some(ref report) = diff_report
        && report.baseline_available
    {
        for u in &report.unused_approvals {
            let page_str = u.page.map(|p| p.to_string()).unwrap_or_else(|| "*".into());
            eprintln!(
                "corpus_sweep: warning: unused approval rule on line {}: {} {} {} {}",
                u.line_number,
                u.corpus,
                u.relative_path,
                page_str,
                u.kind.as_str()
            );
        }
        if report.unapproved_count > 0 {
            eprintln!(
                "corpus_sweep: {} unapproved differences found ({} approved)",
                report.unapproved_count, report.approved_count
            );
            return Err(format!(
                "{} unapproved differences found",
                report.unapproved_count
            ));
        } else {
            println!(
                "corpus_sweep: differential comparison passed ({} approved differences, 0 unapproved)",
                report.approved_count
            );
        }
    }

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
    let mut withhold_names = false;
    let mut baseline_dir = None;
    let mut approved_file = None;

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
            "--withhold-names" => {
                withhold_names = true;
            }
            "--baseline" => {
                let val = args.next().ok_or("--baseline requires a directory path")?;
                baseline_dir = Some(PathBuf::from(val));
            }
            "--approved" => {
                let val = args.next().ok_or("--approved requires a file path")?;
                approved_file = Some(PathBuf::from(val));
            }
            "-h" | "--help" => {
                println!(
                    "Usage: corpus_sweep <corpus_dir> <output_dir> [--file-seconds 60] [--expect-files N] [--worker <path>] [--corpus <name>] [--withhold-names] [--baseline <dir>] [--approved <tsv>]"
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
            "usage: corpus_sweep <corpus_dir> <output_dir> [--file-seconds 60] [--expect-files N] [--baseline <dir>] [--approved <tsv>]"
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
        withhold_names,
        baseline_dir.as_deref(),
        approved_file.as_deref(),
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
            page_hashes: Vec::new(),
        };
        let line = res.to_json_line(false);
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
                page_hashes: Vec::new(),
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
                page_hashes: Vec::new(),
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
                page_hashes: Vec::new(),
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
                page_hashes: Vec::new(),
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

        let md = summary.generate_markdown(false, None);
        assert!(md.contains("| `qpdf` | 2 | 1 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |"));
        assert!(md.contains("| `pdfjs` | 2 | 0 | 0 | 0 | 0 | 1 | 1 | 0 | 0 | 0 |"));
        assert!(md.contains("`Corrupt`"));
        assert!(md.contains("crash.pdf"));

        let json = summary.generate_json(None);
        assert!(json.contains("\"total_files\": 4"));
        assert!(json.contains("\"worker_crashed\": 1"));
        assert!(json.contains("\"worker_timeout\": 1"));
    }

    #[test]
    fn test_withhold_names() {
        let items = vec![
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "secret_crash.pdf".into(),
                bytes: 111,
                pages: Some(1),
                result: SweepResultKind::WorkerCrashed,
                stage: SweepStage::Render,
                page_index: Some(0),
                error_code: Some("Crashed".into()),
                elapsed_ms: 1234,
                peak_memory_bytes: Some(123456),
                page_hashes: vec![PageHashRecord {
                    corpus: "pdfjs".into(),
                    relative_path: "secret_crash.pdf".into(),
                    page_index: 0,
                    render_hash: "render1".into(),
                    text_hash: "text1".into(),
                    dhash: "0000000000000001".into(),
                }],
            },
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "secret_proto.pdf".into(),
                bytes: 222,
                pages: Some(2),
                result: SweepResultKind::ProtocolViolation,
                stage: SweepStage::Open,
                page_index: None,
                error_code: Some("ProtocolViolation".into()),
                elapsed_ms: 2345,
                peak_memory_bytes: Some(234567),
                page_hashes: Vec::new(),
            },
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "secret_timeout.pdf".into(),
                bytes: 333,
                pages: Some(3),
                result: SweepResultKind::WorkerTimeout,
                stage: SweepStage::Render,
                page_index: Some(1),
                error_code: Some("Timeout".into()),
                elapsed_ms: 30000,
                peak_memory_bytes: Some(345678),
                page_hashes: Vec::new(),
            },
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "secret_file_timeout.pdf".into(),
                bytes: 444,
                pages: Some(4),
                result: SweepResultKind::FileTimeout,
                stage: SweepStage::FileTimeout,
                page_index: Some(2),
                error_code: Some("FileTimeout".into()),
                elapsed_ms: 60000,
                peak_memory_bytes: Some(456789),
                page_hashes: Vec::new(),
            },
            FileResult {
                corpus: "qpdf".into(),
                relative_path: "render_failed_visible.pdf".into(),
                bytes: 555,
                pages: Some(5),
                result: SweepResultKind::RenderFailed,
                stage: SweepStage::Render,
                page_index: Some(3),
                error_code: Some("Internal".into()),
                elapsed_ms: 50,
                peak_memory_bytes: Some(567890),
                page_hashes: Vec::new(),
            },
        ];

        // 1. Verify to_json_line with withhold_names = true
        for (i, res) in items.iter().enumerate() {
            let line = res.to_json_line(true);
            if res.result.is_withheld() {
                assert!(
                    line.contains("\"relative_path\":\"(withheld)\""),
                    "item {i} should withhold relative_path"
                );
                assert!(
                    line.contains("\"bytes\":null"),
                    "item {i} should withhold bytes"
                );
                assert!(
                    line.contains("\"pages\":null"),
                    "item {i} should withhold pages"
                );
                assert!(
                    line.contains("\"page_index\":null"),
                    "item {i} should withhold page_index"
                );
                assert!(
                    line.contains("\"error_code\":null"),
                    "item {i} should withhold error_code"
                );
                assert!(
                    line.contains("\"elapsed_ms\":null"),
                    "item {i} should withhold elapsed_ms"
                );
                assert!(
                    line.contains("\"peak_memory_bytes\":null"),
                    "item {i} should withhold peak_memory_bytes"
                );
                assert!(
                    !line.contains(&res.relative_path),
                    "item {i} should not contain original filename: {}",
                    res.relative_path
                );
            } else {
                // render_failed must still show filename and details
                assert!(
                    line.contains("\"relative_path\":\"render_failed_visible.pdf\""),
                    "render_failed must show filename"
                );
                assert!(
                    line.contains("\"bytes\":555"),
                    "render_failed must show bytes"
                );
                assert!(
                    line.contains("\"result\":\"render_failed\""),
                    "render_failed result kind"
                );
            }
        }

        // 2. Verify generate_markdown with withhold_names = true
        let summary = SweepSummary::from_results(&items, 95000);
        let md = summary.generate_markdown(true, None);

        assert!(!md.contains("secret_crash.pdf"));
        assert!(!md.contains("secret_proto.pdf"));
        assert!(!md.contains("secret_timeout.pdf"));
        assert!(!md.contains("secret_file_timeout.pdf"));
        assert!(
            md.contains("render_failed_visible.pdf"),
            "render_failed filename must be preserved in markdown"
        );
        assert!(
            md.contains("(withheld)"),
            "withheld items must show (withheld) placeholder"
        );
    }

    #[test]
    fn test_identical_baseline_and_current() {
        let rec = PageHashRecord {
            corpus: "pdf-differences".into(),
            relative_path: "001.pdf".into(),
            page_index: 0,
            render_hash: "aaaa1111".into(),
            text_hash: "bbbb2222".into(),
            dhash: "0000000000000001".into(),
        };
        let baseline = vec![rec.clone()];
        let current = vec![rec];
        let report = compare_page_hashes(&baseline, &current, Vec::new(), &BTreeSet::new());
        assert_eq!(report.baseline_page_count, 1);
        assert_eq!(report.current_page_count, 1);
        assert_eq!(report.changed_pages_count, 0);
        assert_eq!(report.approved_count, 0);
        assert_eq!(report.unapproved_count, 0);
        assert!(report.differences.is_empty());
    }

    #[test]
    fn test_text_only_changed() {
        let b_rec = PageHashRecord {
            corpus: "pdf-differences".into(),
            relative_path: "001.pdf".into(),
            page_index: 0,
            render_hash: "render_same".into(),
            text_hash: "text_old".into(),
            dhash: "0000000000000001".into(),
        };
        let c_rec = PageHashRecord {
            corpus: "pdf-differences".into(),
            relative_path: "001.pdf".into(),
            page_index: 0,
            render_hash: "render_same".into(),
            text_hash: "text_new".into(),
            dhash: "0000000000000001".into(),
        };
        let report = compare_page_hashes(&[b_rec], &[c_rec], Vec::new(), &BTreeSet::new());
        assert_eq!(report.changed_pages_count, 1);
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences[0].kind, DiffKind::Text);
        assert_eq!(report.differences[0].page_number, 1);
        assert_eq!(report.differences[0].dhash_distance, None);
        assert_eq!(report.unapproved_count, 1);
        assert_eq!(report.approved_count, 0);
    }

    #[test]
    fn test_render_only_changed() {
        let b_rec = PageHashRecord {
            corpus: "qpdf".into(),
            relative_path: "a.pdf".into(),
            page_index: 0,
            render_hash: "render_old".into(),
            text_hash: "text_same".into(),
            dhash: "0000000000000000".into(),
        };
        let c_rec = PageHashRecord {
            corpus: "qpdf".into(),
            relative_path: "a.pdf".into(),
            page_index: 0,
            render_hash: "render_new".into(),
            text_hash: "text_same".into(),
            dhash: "0000000000000003".into(), // Hamming distance = 2 (bits 0 and 1)
        };
        let report = compare_page_hashes(&[b_rec], &[c_rec], Vec::new(), &BTreeSet::new());
        assert_eq!(report.changed_pages_count, 1);
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.differences[0].kind, DiffKind::Render);
        assert_eq!(report.differences[0].page_number, 1);
        assert_eq!(report.differences[0].dhash_distance, Some(2));
        assert_eq!(report.unapproved_count, 1);
    }

    #[test]
    fn test_fewer_pages() {
        let b1 = PageHashRecord {
            corpus: "qpdf".into(),
            relative_path: "multi.pdf".into(),
            page_index: 0,
            render_hash: "r0".into(),
            text_hash: "t0".into(),
            dhash: "0000000000000000".into(),
        };
        let b2 = PageHashRecord {
            corpus: "qpdf".into(),
            relative_path: "multi.pdf".into(),
            page_index: 1,
            render_hash: "r1".into(),
            text_hash: "t1".into(),
            dhash: "0000000000000000".into(),
        };
        let c1 = b1.clone();
        // Baseline has 2 pages, current has only page 0
        let report = compare_page_hashes(&[b1, b2], &[c1], Vec::new(), &BTreeSet::new());
        assert_eq!(report.changed_pages_count, 1);
        // Page 2 missing generates both text and render differences
        assert_eq!(report.differences.len(), 2);
        assert_eq!(report.differences[0].page_number, 2);
        assert_eq!(report.differences[0].kind, DiffKind::Render);
        assert_eq!(report.differences[1].page_number, 2);
        assert_eq!(report.differences[1].kind, DiffKind::Text);
        assert_eq!(report.unapproved_count, 2);
    }

    #[test]
    fn test_approved_single_page() {
        let b = PageHashRecord {
            corpus: "pdfjs".into(),
            relative_path: "issue.pdf".into(),
            page_index: 2,
            render_hash: "r_old".into(),
            text_hash: "t_same".into(),
            dhash: "0000000000000000".into(),
        };
        let c = PageHashRecord {
            corpus: "pdfjs".into(),
            relative_path: "issue.pdf".into(),
            page_index: 2,
            render_hash: "r_new".into(),
            text_hash: "t_same".into(),
            dhash: "0000000000000001".into(),
        };
        let approvals = vec![ApprovedEntry {
            corpus: "pdfjs".into(),
            relative_path: "issue.pdf".into(),
            page: Some(3), // 1-based page 3 (index 2)
            kind: DiffKind::Render,
            reason: "anti-aliasing change".into(),
            pr: "#617".into(),
            line_number: 10,
            match_count: 0,
        }];
        let report = compare_page_hashes(&[b], &[c], approvals, &BTreeSet::new());
        assert_eq!(report.changed_pages_count, 1);
        assert_eq!(report.approved_count, 1);
        assert_eq!(report.unapproved_count, 0);
        assert_eq!(
            report.differences[0].approved_by,
            Some(("anti-aliasing change".into(), "#617".into()))
        );
        assert!(report.unused_approvals.is_empty());
    }

    #[test]
    fn test_approved_wildcards() {
        let b1 = PageHashRecord {
            corpus: "pdf-differences".into(),
            relative_path: "file1.pdf".into(),
            page_index: 0,
            render_hash: "r1_old".into(),
            text_hash: "t1".into(),
            dhash: "0".into(),
        };
        let b2 = PageHashRecord {
            corpus: "pdf-differences".into(),
            relative_path: "file1.pdf".into(),
            page_index: 1,
            render_hash: "r2_old".into(),
            text_hash: "t2".into(),
            dhash: "0".into(),
        };
        let b3 = PageHashRecord {
            corpus: "pdf-differences".into(),
            relative_path: "file2.pdf".into(),
            page_index: 0,
            render_hash: "r3_old".into(),
            text_hash: "t3".into(),
            dhash: "0".into(),
        };

        let c1 = PageHashRecord {
            render_hash: "r1_new".into(),
            ..b1.clone()
        };
        let c2 = PageHashRecord {
            render_hash: "r2_new".into(),
            ..b2.clone()
        };
        let c3 = PageHashRecord {
            render_hash: "r3_new".into(),
            ..b3.clone()
        };

        // Approval with wildcard page (*) for file1.pdf, and wildcard path/page for file2
        let approvals = vec![
            ApprovedEntry {
                corpus: "pdf-differences".into(),
                relative_path: "file1.pdf".into(),
                page: None, // * wildcard covers both page 1 and page 2
                kind: DiffKind::Render,
                reason: "file1 update".into(),
                pr: "#617".into(),
                line_number: 5,
                match_count: 0,
            },
            ApprovedEntry {
                corpus: "pdf-differences".into(),
                relative_path: "*".into(), // * wildcard covers file2
                page: None,
                kind: DiffKind::Render,
                reason: "all corpus render change".into(),
                pr: "#617".into(),
                line_number: 6,
                match_count: 0,
            },
        ];

        let report = compare_page_hashes(&[b1, b2, b3], &[c1, c2, c3], approvals, &BTreeSet::new());
        assert_eq!(report.changed_pages_count, 3);
        assert_eq!(report.approved_count, 3);
        assert_eq!(report.unapproved_count, 0);
    }

    #[test]
    fn test_unused_approval_rule() {
        let b = PageHashRecord {
            corpus: "qpdf".into(),
            relative_path: "exact.pdf".into(),
            page_index: 0,
            render_hash: "r".into(),
            text_hash: "t".into(),
            dhash: "0".into(),
        };
        let c = b.clone();
        let approvals = vec![ApprovedEntry {
            corpus: "qpdf".into(),
            relative_path: "other.pdf".into(),
            page: Some(1),
            kind: DiffKind::Text,
            reason: "never used".into(),
            pr: "#123".into(),
            line_number: 4,
            match_count: 0,
        }];

        let report = compare_page_hashes(&[b], &[c], approvals, &BTreeSet::new());
        assert_eq!(report.changed_pages_count, 0);
        assert_eq!(report.unapproved_count, 0);
        assert_eq!(report.unused_approvals.len(), 1);
        assert_eq!(report.unused_approvals[0].line_number, 4);

        let md = report.generate_markdown();
        assert!(md.contains("未使用的核准規則"));
        assert!(md.contains("第 4 行"));
    }

    #[test]
    fn test_withhold_categories_no_hashes() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_withhold_{}",
            Instant::now().elapsed().as_nanos()
        ));
        let _ = fs::create_dir_all(&temp_dir);

        let crashed_file = FileResult {
            corpus: "pdfjs".into(),
            relative_path: "crash.pdf".into(),
            bytes: 100,
            pages: Some(2),
            result: SweepResultKind::WorkerCrashed,
            stage: SweepStage::Render,
            page_index: Some(1),
            error_code: Some("Crashed".into()),
            elapsed_ms: 10,
            peak_memory_bytes: None,
            page_hashes: vec![PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "crash.pdf".into(),
                page_index: 0,
                render_hash: "r0".into(),
                text_hash: "t0".into(),
                dhash: "0".into(),
            }],
        };

        let normal_file = FileResult {
            corpus: "pdfjs".into(),
            relative_path: "normal.pdf".into(),
            bytes: 200,
            pages: Some(1),
            result: SweepResultKind::Ok,
            stage: SweepStage::Render,
            page_index: None,
            error_code: None,
            elapsed_ms: 10,
            peak_memory_bytes: None,
            page_hashes: vec![PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "normal.pdf".into(),
                page_index: 0,
                render_hash: "rn0".into(),
                text_hash: "tn0".into(),
                dhash: "0".into(),
            }],
        };

        let results = vec![crashed_file, normal_file];
        let hashes_path = temp_dir.join("page-hashes.jsonl");
        let mut writer = BufWriter::new(fs::File::create(&hashes_path).unwrap());
        for res in &results {
            if !(true && res.result.is_withheld()) {
                for ph in &res.page_hashes {
                    writeln!(writer, "{}", ph.to_json_line()).unwrap();
                }
            }
        }
        writer.flush().unwrap();

        let content = fs::read_to_string(&hashes_path).unwrap();
        assert!(!content.contains("crash.pdf"));
        assert!(!content.contains("r0"));
        assert!(content.contains("normal.pdf"));
        assert!(content.contains("rn0"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_no_baseline() {
        let report = DifferentialReport {
            baseline_available: false,
            ..Default::default()
        };
        let md = report.generate_markdown();
        assert!(md.contains("沒有基準，這次只記錄"));
    }

    #[test]
    fn test_dhash_and_hamming_distance() {
        // Create an all-white 100x100 raster (RGBA8)
        let white_pixels = vec![255u8; 100 * 100 * 4];
        let white_raster = ipc_contract::worker::Raster {
            width: 100,
            height: 100,
            pixels: white_pixels,
        };
        let h_white = compute_dhash(&white_raster);

        // Same raster produces identical dhash
        let h_white2 = compute_dhash(&white_raster);
        assert_eq!(h_white, h_white2);
        assert_eq!(hamming_distance(h_white, h_white2), 0);

        // Half black, half white raster
        let mut split_pixels = vec![0u8; 100 * 100 * 4];
        for y in 0..100 {
            for x in 50..100 {
                let off = (y * 100 + x) * 4;
                split_pixels[off] = 255;
                split_pixels[off + 1] = 255;
                split_pixels[off + 2] = 255;
                split_pixels[off + 3] = 255;
            }
        }
        let split_raster = ipc_contract::worker::Raster {
            width: 100,
            height: 100,
            pixels: split_pixels,
        };
        let h_split = compute_dhash(&split_raster);
        assert_ne!(h_white, h_split);
        assert!(hamming_distance(h_white, h_split) > 0);
    }

    #[test]
    fn test_differential_withhold_names_hides_crashed_baseline_file() {
        let baseline = vec![
            PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "secret_crash_in_baseline.pdf".into(),
                page_index: 0,
                render_hash: "b_r0_secret".into(),
                text_hash: "b_t0_secret".into(),
                dhash: "0000000000000001".into(),
            },
            PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "secret_crash_in_baseline.pdf".into(),
                page_index: 1,
                render_hash: "b_r1_secret".into(),
                text_hash: "b_t1_secret".into(),
                dhash: "0000000000000002".into(),
            },
            PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "normal.pdf".into(),
                page_index: 0,
                render_hash: "r_ok".into(),
                text_hash: "t_ok".into(),
                dhash: "0000000000000003".into(),
            },
        ];

        let results = vec![
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "secret_crash_in_baseline.pdf".into(),
                bytes: 100,
                pages: Some(2),
                result: SweepResultKind::WorkerCrashed,
                stage: SweepStage::Render,
                page_index: Some(0),
                error_code: Some("Crashed".into()),
                elapsed_ms: 42,
                peak_memory_bytes: None,
                page_hashes: Vec::new(),
            },
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "normal.pdf".into(),
                bytes: 200,
                pages: Some(1),
                result: SweepResultKind::Ok,
                stage: SweepStage::Render,
                page_index: None,
                error_code: None,
                elapsed_ms: 10,
                peak_memory_bytes: None,
                page_hashes: vec![baseline[2].clone()],
            },
        ];

        let report = compare_results_against_baseline(&baseline, &results, Vec::new(), true);
        assert_eq!(report.withheld_files_count, 1);
        assert_eq!(report.unapproved_count, 1);
        assert!(report.differences.is_empty());

        let summary = SweepSummary::from_results(&results, 100);
        let md = summary.generate_markdown(true, Some(&report));
        assert!(
            !md.contains("secret_crash_in_baseline.pdf"),
            "withheld file path must not appear in summary.md:\n{md}"
        );
        assert!(!md.contains("b_r0_secret"));
        assert!(!md.contains("b_t0_secret"));
        assert!(!md.contains("b_r1_secret"));
        assert!(!md.contains("b_t1_secret"));
        assert!(
            md.contains("1 個檔案這次異常，名稱不公開"),
            "summary.md must report withheld count:\n{md}"
        );
        assert!(
            !md.contains("未發現任何與基準不同的頁面"),
            "summary.md must not claim no differences when withheld files failed:\n{md}"
        );

        let json = summary.generate_json(Some(&report));
        assert!(json.contains("\"withheld_files\": 1"));
        assert!(json.contains("\"unapproved\": 1"));
        assert!(!json.contains("secret_crash_in_baseline.pdf"));

        // Even wildcard approvals must not suppress a withheld crash failure.
        let wildcard_approvals = vec![
            ApprovedEntry {
                corpus: "pdfjs".into(),
                relative_path: "*".into(),
                page: None,
                kind: DiffKind::Text,
                reason: "wildcard text".into(),
                pr: "#999".into(),
                line_number: 1,
                match_count: 0,
            },
            ApprovedEntry {
                corpus: "pdfjs".into(),
                relative_path: "*".into(),
                page: None,
                kind: DiffKind::Render,
                reason: "wildcard render".into(),
                pr: "#999".into(),
                line_number: 2,
                match_count: 0,
            },
        ];
        let report_with_wildcards =
            compare_results_against_baseline(&baseline, &results, wildcard_approvals, true);
        assert_eq!(report_with_wildcards.withheld_files_count, 1);
        assert_eq!(report_with_wildcards.unapproved_count, 1);
        assert!(report_with_wildcards.differences.is_empty());
    }

    #[test]
    fn test_differential_without_withhold_names_lists_crashed_baseline_file() {
        let baseline = vec![
            PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "secret_crash_in_baseline.pdf".into(),
                page_index: 0,
                render_hash: "b_r0_secret".into(),
                text_hash: "b_t0_secret".into(),
                dhash: "0000000000000001".into(),
            },
            PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "secret_crash_in_baseline.pdf".into(),
                page_index: 1,
                render_hash: "b_r1_secret".into(),
                text_hash: "b_t1_secret".into(),
                dhash: "0000000000000002".into(),
            },
            PageHashRecord {
                corpus: "pdfjs".into(),
                relative_path: "normal.pdf".into(),
                page_index: 0,
                render_hash: "r_ok".into(),
                text_hash: "t_ok".into(),
                dhash: "0000000000000003".into(),
            },
        ];

        let results = vec![
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "secret_crash_in_baseline.pdf".into(),
                bytes: 100,
                pages: Some(2),
                result: SweepResultKind::WorkerCrashed,
                stage: SweepStage::Render,
                page_index: Some(0),
                error_code: Some("Crashed".into()),
                elapsed_ms: 42,
                peak_memory_bytes: None,
                page_hashes: Vec::new(),
            },
            FileResult {
                corpus: "pdfjs".into(),
                relative_path: "normal.pdf".into(),
                bytes: 200,
                pages: Some(1),
                result: SweepResultKind::Ok,
                stage: SweepStage::Render,
                page_index: None,
                error_code: None,
                elapsed_ms: 10,
                peak_memory_bytes: None,
                page_hashes: vec![baseline[2].clone()],
            },
        ];

        let report = compare_results_against_baseline(&baseline, &results, Vec::new(), false);
        assert_eq!(report.withheld_files_count, 0);
        assert_eq!(report.unapproved_count, 4);
        assert_eq!(report.differences.len(), 4);

        let summary = SweepSummary::from_results(&results, 100);
        let md = summary.generate_markdown(false, Some(&report));
        assert!(
            md.contains("secret_crash_in_baseline.pdf"),
            "without --withhold-names, summary.md should list the path:\n{md}"
        );
        assert!(!md.contains("名稱不公開"));
    }
}
