# MuPDF 綁定與建置

對應工作卡 MVP-03、ADR 0003。記錄 MuPDF 如何進入專案、如何建置、關掉了什麼，以及還沒解決的事。

## 選擇

| 選項 | 優點 | 缺點 | 結論 |
|---|---|---|---|
| **`mupdf` crate（messense/mupdf-rs）** | 安全的 Rust API；以 Cargo feature 在編譯期關閉 JavaScript 等功能；維護中（2026-06 發布 0.8.0，約 170 萬次下載） | 版本跟隨 crate 發布；Windows 建置需要 LLVM | ✅ 採用 |
| 自行以 bindgen 包裝 MuPDF C API | 完全掌控 | 要自己維護 unsafe FFI 與建置腳本，工具鏈需求相同 | ✗ |
| 使用預先編譯的 MuPDF DLL | 建置快 | 來源無法驗證的二進位檔（AGENTS.md 禁止） | ✗ |

## 版本與來源

| 項目 | 值 |
|---|---|
| Rust crate | `mupdf = "=0.8.0"`、`mupdf-sys 0.8.0`（精確固定） |
| MuPDF | 1.27.2（原始碼隨 `mupdf-sys` crate 發布） |
| 取得方式 | crates.io；`Cargo.lock` 記錄 SHA-256，cargo 下載時驗證 |
| 授權 | `mupdf`、`mupdf-sys` 與 MuPDF 皆為 AGPL-3.0（或向 Artifex 購買商業授權），見 DEC-02（#2） |

升級 MuPDF 一律視為安全變更（`needs-security-review`）。**cargo-deny 的弱點資料庫只涵蓋 Rust crate，不涵蓋 MuPDF 的 C 程式碼**；升級前與每次發布前，要人工查 Artifex 的版本說明與 NVD 上 MuPDF 的 CVE。

## 編譯期功能

`crates/pdf_worker/Cargo.toml`：`default-features = false, features = ["base14-fonts"]`。

| 功能 | 狀態 | 說明 |
|---|---|---|
| JavaScript（`js`） | **關閉** | 以 `FZ_ENABLE_JS=0` 編譯，MuPDF 的腳本支援只剩空殼。測試 `javascript_is_compiled_out` 在測試碼中刻意呼叫 `enable_js()`，確認 `is_js_supported()` 仍為 false。正式程式碼不得呼叫 `enable_js`（ADR 0001 的表單腳本沙盒日後另行評估） |
| 標準 14 字型（`base14-fonts`） | 開啟 | 沒有它，使用未嵌入標準字型（Helvetica 等）的 PDF 會顯示成空白 |
| XPS、SVG、CBZ、圖片、HTML、EPUB | 關閉 | 只處理 PDF |
| Tesseract OCR、DOCX 輸出、Brotli | 關閉 | 不在 MVP；之後的 OCR 另立 ADR |
| 系統字型（`system-fonts`） | 關閉 | 見下方「CJK 字型」 |

注意：Windows 上 `mupdf-sys` 用 MSBuild 建置 MuPDF 的 Visual Studio 方案，會編譯方案內所有第三方函式庫（包含 Tesseract 等）的原始碼；Cargo feature 以 `FZ_ENABLE_*` 決定 MuPDF 是否使用它們，未被參照的程式碼不會連結進執行檔。這主要影響建置時間。

## 建置需求（Windows）

| 工具 | 用途 |
|---|---|
| Visual Studio Build Tools（C++ 桌面開發） | MSBuild 建置 MuPDF；VS 2019 用 v142、VS 2022 用 v143 工具組（自動偵測） |
| LLVM（`winget install LLVM.LLVM`） | bindgen 需要 libclang；另外用 clang-cl 編譯 `mupdf-sys` 的小型 C wrapper |

`.cargo/config.toml` 設定了三個環境變數（可被系統環境變數覆寫）：

- `CC_x86_64_pc_windows_msvc` → clang-cl：`mupdf-sys` 的 wrapper 使用空初始化子 `{}`，VS 2019 的 C 編譯器（MSVC 14.29）不接受，clang-cl 在任何 VS 版本下都能以 MSVC ABI 編譯。
- `CLANG_PATH`、`LIBCLANG_PATH`：讓 bindgen 使用 clang 自己的標頭檔。若 LLVM 不在 `PATH` 上，bindgen 會默默改用 MSVC 的 C 標頭檔，缺少 `max_align_t`，導致 `mupdf` crate 編譯失敗。

路徑是 LLVM 的預設安裝位置，GitHub 的 Windows runner 也在同一位置。

## 建置時間

| 情境 | 時間 |
|---|---|
| 負責人的電腦，乾淨建置 `mupdf-sys`（debug） | 約 2 分 30 秒 |
| 負責人的電腦，只改 `pdf_worker` 後重新建置 | 約 1 秒 |
| CI `Rust (Windows)`，無快取 | 見 PR 說明 |

CI 由 `Swatinem/rust-cache` 快取 `target/`（含 MuPDF 建置結果），`Cargo.lock` 不變時不會重建 MuPDF。

## 目前的 API（`crates/pdf_worker/src/engine.rs`）

- `PdfDocument::from_bytes`：檢查前 1024 bytes 內有 `%PDF-`（否則 `NotPdf`）；加密文件回傳 `Encrypted`（MVP 不支援）。
- `page_count`、`page_size`（point，未旋轉）。
- `render(index, scale, rotation)`：縮放 0.01～64、旋轉 0／90／180／270（順時針）；**在 MuPDF 配置記憶體之前**先以頁面尺寸估算點陣圖大小，超過 4096 × 4096 像素就拒絕；輸出不透明 RGBA8（與 IPC 合約一致）。
- 我們的程式碼沒有任何 `unsafe`（workspace 的 `unsafe_code = "deny"` 維持有效），FFI 全部封裝在 `mupdf` crate 內。

## 尚未解決

- **CJK 備援字型**：未嵌入字型的中文 PDF（例如 `tests/corpus/benign/mixed-text-zh-en.pdf`）目前沒有字形可畫，文字抽取與搜尋不受影響。選項：打包 Noto CJK（體積大）或開啟 `system-fonts`（worker 需要讀取 `C:\Windows\Fonts`，影響 MVP-04 的沙盒設計；規格 §4 也偏好使用本機字型）。需要決策，已另開 Issue。
- **AES-256 與簽章樣本**：MuPDF 可用後補（#27）。
- **worker 隔離**：本文件只涵蓋引擎本身；行程隔離、handle 交付與沙盒見 MVP-04。
