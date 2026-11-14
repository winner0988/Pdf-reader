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
| 取得方式 | `mupdf`、`mupdf-fonts-droid`：crates.io，`Cargo.lock` 記錄 SHA-256，cargo 下載時驗證。`mupdf-sys`：git（見下方「`mupdf-sys` 的來源」），`Cargo.lock` 記錄 commit |
| 授權 | `mupdf`、`mupdf-sys` 與 MuPDF 皆為 AGPL-3.0（或向 Artifex 購買商業授權），見 DEC-02（#2） |

升級 MuPDF 一律視為安全變更（`needs-security-review`）。**cargo-deny 的弱點資料庫只涵蓋 Rust crate，不涵蓋 MuPDF 的 C 程式碼**，也不涵蓋 MuPDF 內附的第三方函式庫（OpenJPEG、FreeType 等）；升級前與每次發布前，要人工查 Artifex 的版本說明與 NVD 上 MuPDF 的 CVE。內嵌第三方 C 函式庫的完整清單、版本與更新方式見 [bundled-libraries.md](../security/bundled-libraries.md)（#553）。

### `mupdf-sys` 的來源

`mupdf-sys` 0.8.0 內附 MuPDF 1.27.2，而 1.27.2 內附的 OpenJPEG 少了上游已經加上的一個檢查：`opj_j2k_read_sod` 寫入 tile-part 索引前沒有檢查範圍。

- 上游的修正：OpenJPEG [`91d08b11`](https://github.com/uclouvain/openjpeg/commit/91d08b11a72764f6d199f32fa0c1b1abd4edc2ad)；Artifex 的 OpenJPEG 是 `328a8da`，MuPDF 1.28.0 起包含。
- crates.io 上還沒有內附 MuPDF 1.28 以上的 `mupdf-sys`，所以暫時從 git 取得：
  - 來源是 <https://github.com/winner0988/mupdf-rs> 的 `v0.8.0-openjpeg-read-sod` 分支，也就是上游 `messense/mupdf-rs` 的 `v0.8.0` 加上一個 commit；
  - 那個 commit 只改 `mupdf-sys/build.rs`：建置時把上游的修正原文套到複製出來的 OpenJPEG 原始碼。原始碼裡既找不到未修正的寫法、也找不到修正後的寫法時，建置失敗，不會默默建出沒有檢查的版本；
  - 根目錄與 `fuzz/` 的 `Cargo.toml` 以 `[patch.crates-io]` 指向同一個 `rev`（完整的 commit），`deny.toml` 的 `allow-git` 只允許這一個 repo；
  - `mupdf` 與字型 crate 仍然來自 crates.io：上游 repo 裡的字型檔是符號連結，Windows 上取出來會壞掉。
- 代價：cargo 第一次要下載 MuPDF 與它所有第三方函式庫的 git 歷史（約 850 MB），之後存在 `~/.cargo/git`。
- **移除條件**：`mupdf-sys` 發布內附 MuPDF 1.28.0 以上的版本後，在升級的 PR 裡刪掉兩個 `[patch.crates-io]` 與 `allow-git` 的那一項，這一節也改回 crates.io。
- 要換 `rev`（例如上游再有修正）：在 fork 的分支加 commit，兩個 `Cargo.toml` 改成新的完整 commit，再更新兩個 `Cargo.lock`。這是依賴來源的變更，要 `needs-security-review`。

## 編譯期功能

`crates/pdf_worker/Cargo.toml`：`default-features = false, features = ["base14-fonts", "bundled-fonts-droid", "epub", "html", "svg", "xps"]`（後四個是 B3-03 加的，見 [create-pdf.md](create-pdf.md)）。

| 功能 | 狀態 | 說明 |
|---|---|---|
| JavaScript（`js`） | **關閉** | 以 `FZ_ENABLE_JS=0` 編譯，MuPDF 的腳本支援只剩空殼。測試 `javascript_is_compiled_out` 在測試碼中刻意呼叫 `enable_js()`，確認 `is_js_supported()` 仍為 false。正式程式碼不得呼叫 `enable_js`（ADR 0001 的表單腳本沙盒日後另行評估） |
| 標準 14 字型（`base14-fonts`） | 開啟 | 沒有它，使用未嵌入標準字型（Helvetica 等）的 PDF 會顯示成空白 |
| Droid CJK 備援字型（`bundled-fonts-droid`） | 開啟 | 見下方「CJK 字型」 |
| HTML、EPUB、SVG、XPS（`html`、`epub`、`svg`、`xps`） | 開啟（B3-03） | 「從檔案建立 PDF」要讀的格式。每一項就是 worker 沙盒裡多一個解析器，所以有 fuzz 目標（`create_pdf`）與惡意樣本；轉換是唯一有意用到它們的地方。**開啟文件時仍然只收 PDF**：worker 要求 `%PDF-` 檔頭、以 PDF 的 MIME 開啟，結果不是 PDF 文件就拒絕。但 MuPDF 也看內容選讀取器，所以一個同時有 `%PDF-` 檔頭又是 EPUB／XPS 的混合檔，理論上會先被這些讀取器解析一部分才被拒絕；原本就編進來的 Office、txt、fb2、mobi 讀取器早就是這樣，風險在沙盒內，`open_document` 的 fuzz 目標涵蓋這條路。評估與理由見 [create-pdf.md](create-pdf.md) |
| CBZ、圖片文件（`cbz`、`img`） | 關閉 | 「把圖片當成文件開啟」不需要：建立 PDF 時圖片是以 `Image` 加進頁面，與這兩個開關無關 |
| Tesseract OCR | Windows 上一律建置 | `tesseract` 功能只影響 make 建置；Windows 的 Visual Studio 方案一律以 `HAVE_TESSERACT` 編譯 MuPDF（見下方的注意）。OCR 見 ADR 0015 |
| DOCX 輸出、Brotli | 關閉 | 不在 MVP。匯出 Word（B3-02）沒有用 MuPDF 的 DOCX 寫出器：繫結的 `DocumentWriter` 只收檔案路徑，worker 沒有路徑（ADR 0008）；改由 worker 以結構化文字自己組出 `.docx`，見 [docx-export.md](docx-export.md) |
| 系統字型（`system-fonts`） | 關閉 | worker 在沙盒中不讀取系統的字型檔 |
| Noto、SIL 字型（`bundled-fonts-noto`、`bundled-fonts-sil`） | 關閉 | 非 CJK 文字系統的備援字型，目前不需要 |

## CJK 字型

PDF 可以使用中日韓字型而不嵌入它（例如只寫 `/BaseFont /MingLiU` 與 CNS1 字元集）。這時 MuPDF 需要一個替代字型；**找不到就無法載入這個字型**：那段文字既畫不出來，也抽取不到，所以搜尋也找不到。

- **決定**：打包 Droid CJK 備援字型（DEC-03，[#31](https://github.com/winner0988/Pdf-reader/issues/31)，負責人 2026-09-24 決定）。
- **來源與授權**：
  - `mupdf-fonts-droid` crate，與 `mupdf` 同一個 repo、同一版本；
  - 字型本身是 Droid Sans Fallback，Apache-2.0。
  - 發布安裝檔時，第三方授權聲明要包含它（ADR 0011）。
- **運作方式**：
  - `mupdf` crate 建立 context 時就向 MuPDF 註冊字型回呼；
  - 需要 CJK 字型時，從編進 `pdf_worker.exe` 的資料提供，**不讀取任何檔案**，所以 worker 的沙盒不需要任何例外。
- **代價**：
  - crate 內含 DroidSansFallback（3.6 MB）與 DroidSansFallbackFull（5.1 MB）兩個字型，`pdf_worker.exe` 因此從 7.2 MB 增加到 15.9 MB；
  - 安裝檔以 LZMA 壓縮，兩個字型壓縮後約 1.8 MB。
- **沒有選擇**：
  - 系統字型：worker 要讀取 `C:\Windows\Fonts`，不同電腦的顯示也會不同；
  - 先不處理：中文使用者常見這類 PDF。
- **測試**：`crates/pdf_worker/tests/fonts.rs` 以 `benign/mixed-text-zh-en.pdf` 確認中文可以搜尋，且字形區域確實有筆畫。

注意：Windows 上 `mupdf-sys` 用 MSBuild 建置 MuPDF 的 Visual Studio 方案，會編譯方案內所有第三方函式庫的原始碼，這主要影響建置時間。Cargo feature 以 `FZ_ENABLE_*` 決定 MuPDF 是否使用它們，未被參照的程式碼不會連結進執行檔，但 **Tesseract 與 Leptonica 例外**：

- 方案一律以 `HAVE_TESSERACT`、`HAVE_LEPTONICA` 編譯 MuPDF，沒有 `FZ_ENABLE_*` 可以關閉；
- release 的 `pdf_worker.exe` 因此已經包含它們的一部分：依 link map，至少有 Tesseract 223 個原始檔中的 123 個、Leptonica 182 個中的 37 個，以及 MuPDF 的 `tessocr` 與 PDF OCR 輸出器（`output-pdfocr`）；
- worker 從不呼叫它們：只有明確建立 OCR 裝置或 PDF OCR 輸出器時才會執行；
- ADR 0015（提議中）建議的 OCR 就是使用這一份。

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
- MuPDF 的 FFI 幾乎全部封裝在 `mupdf` crate 內；workspace 的 `unsafe_code = "deny"` 維持有效。唯一的例外是 `crates/pdf_worker/src/owner_password.rs`（#88）：
  - 繫結不說是哪一個密碼開啟了文件，所以這裡以 `mupdf-sys`（與 `mupdf` 同一版本，沒有自己的 feature）另開一次文件，呼叫 `pdf_authenticate_password`；
  - 為了讓 MuPDF 的錯誤（long jump）絕不穿過 Rust 的呼叫框架：開檔經由 `mupdf-sys` 會捕捉錯誤的 C wrapper；其他呼叫都在這個模組自己的 context 上，沒有任何 `fz_try` 區塊，MuPDF 遇到錯誤會直接結束行程而不是跳躍；
  - `FZ_VERSION` 不在 `mupdf-sys` 的繫結中，所以版本寫在程式碼裡。升級 MuPDF 時若沒有更新，MuPDF 不會建立 context，擁有者密碼的測試會失敗。

## 尚未解決

- **worker 隔離**：本文件只涵蓋引擎本身；行程隔離、handle 交付與沙盒見 MVP-04。
