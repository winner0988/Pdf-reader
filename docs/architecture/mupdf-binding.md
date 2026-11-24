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
| MuPDF | 1.28.6（原始碼隨 `mupdf-sys` fork 取得） |
| 取得方式 | `mupdf`、`mupdf-sys`：git（見下方「`mupdf-sys` 的來源」），`Cargo.lock` 記錄 commit。`mupdf-fonts-droid`：crates.io，`Cargo.lock` 記錄 SHA-256，cargo 下載時驗證 |
| 授權 | `mupdf`、`mupdf-sys` 與 MuPDF 皆為 AGPL-3.0（或向 Artifex 購買商業授權），見 DEC-02（#2） |

升級 MuPDF 一律視為安全變更（`needs-security-review`）。**cargo-deny 的弱點資料庫只涵蓋 Rust crate，不涵蓋 MuPDF 的 C 程式碼**，也不涵蓋 MuPDF 內附的第三方函式庫（OpenJPEG、FreeType 等）；發行前的例行對照程序見 [release.md](../release.md#發行前) 的檢查表。內嵌第三方 C 函式庫的完整清單、版本與更新方式見 [bundled-libraries.md](../security/bundled-libraries.md#清單查證與更新程序)（#553）。

### 升級與來源變更規則

凡涉及升級 MuPDF 版本，或變更 `mupdf-sys` 的來源（包括修改 `Cargo.toml` 中 `[patch.crates-io]` 的 `rev`、換 fork 或回到 crates.io）的 PR，必須滿足以下三項條件方可完成：

1. **更新清單與依據**：照新的原始碼更新 [bundled-libraries.md](../security/bundled-libraries.md#清單查證與更新程序) 的版本、commit、查證依據與查驗日期。
2. **重做已知問題對照**：對清單列出的每個第三方函式庫重新到 OSV、NVD 與各專案安全公告或版本說明查核已知問題，對照結果照 [SECURITY.md](../../SECURITY.md) 私下交給負責人（不寫進 PR、commit 或 CHANGELOG）。
3. **比對公開語料掃描差異**：執行公開語料庫完整掃描（`corpus-sweep`），比對與前一版本的每頁文字與渲染雜湊差異；若有預期中的差異（如反鋸齒微調或字型修正），需記錄於 `tests/corpus-sweep/approved-differences.tsv` 並在 PR 說明中列出涵蓋頁數與原因（詳見 [corpus-sweep.md](../security/corpus-sweep.md)）。

三件事都做完，該 PR 才算完成。

### `mupdf-sys` 的來源

- **為什麼用 fork**：crates.io 停在 0.8.0，內附的是 1.27.2。第一次建置時 cargo 要下載 MuPDF 與它所有第三方函式庫的 git 歷史（約 850 MB），之後存在 `~/.cargo/git`。
- **分支與 commit**：來源是 <https://github.com/winner0988/mupdf-rs> 的分支 `v0.8.0-mupdf-1.28`，commit 為 `8520f108e2adc332ad53b7a61281137650275904`。它帶了：
  - 子模組指到 MuPDF 1.28.6（`da5f5eb890d7b3fb76903160b5f7f48ba1301492`）；
  - `src/page.rs` 補上 `table_hunt_options`（`vertically_collapse_bordered_cells: 0`，行為與 1.27.2 相同）；
  - 根目錄 `Cargo.toml` 的字型 crate 拿掉 `path`，改從 crates.io 取；
  - `mupdf-sys/build.rs` 以 `FZ_ENABLE_MD=0` 關掉 1.28 新增的 Markdown 讀取器；
  - Tesseract 的修補：#563 的 5 個、#577 的 22 個，加 #580 的 10 個上游 commit（和 MuPDF 內附的原始碼比，33 個檔案 +549 −80）；原始碼裡既找不到未修正、也找不到修正後的寫法時，建置失敗；MuPDF 開始編譯 Tesseract 的舊引擎或 `FAST_FLOAT` 時也會失敗；
  - FreeType 的修補：#626 套的上游 FreeType 2.15.0 的 12 個 commit（和 MuPDF 內附的 2.14.3 比，6 個檔案 +67 −40）；原始碼裡既找不到未修正、也找不到修正後的寫法時，建置失敗；
  - 文字擷取的修補，是我們自己寫的（上游的 1.28.x 與 master 到 2026-11-23 都還沒修）；和 Tesseract 的修補一樣，找不到未修正、也找不到修正後的寫法時，建置失敗：
    - `source/pdf/pdf-op-run.c`（#628、#638、#678）：1.28 把文字繪製模式 4～7 的字另外累積一份，在 ET 時當成裁切套用，也當成裁切的文字送給裝置。修補之後，這一份一律不帶字元（`-1`，和一個字元對應多個字形時多出來的字形一樣，文字裝置會略過）；模式 7 的字在它那一段送出時，和模式 3 一樣當成看不見的文字送給裝置，所以每個字元都擷取一次，順序和內容相同；ET 不看最後一段的模式，一律套用累積的裁切。修好的問題：
      - 模式 4～6 的字畫出來之後，結構化文字裝置把裁切的那一份又擷取一次（#628）；
      - ET 時看文字物件最後一段的模式決定要不要裁切，文字物件換回模式 0～3 時，裁切與模式 7 的字都不見了（#638）；
      - 一個字形對到好幾個字元時，裁切的那一份只有第一個字元，模式 7 的字少了其他字元，例如 fi 連字的 i（#678）；
    - `source/pdf/pdf-type3.c`（#629）：1.28 把不是標準名稱、以 `g` 開頭的字形名稱當成十六進位的 TrueType 字形編號（為了舊版 Ghostscript 產生的 TrueType 子集，`g0024` 是 `A`），Type3 字型用 `/g32` 這類十進位字元碼命名時就擷取成別的字。Type3 字型不再套用這條推測，和 1.27.2 一樣：字元碼 32～126 沒有標準名稱時，就當成 ASCII；其他字型照舊；
  - tiling pattern 的兩個上限（#550），也是我們自己寫的（上游的 1.28.x 與 master 到 2026-11-22 都還沒有）。碰到上限的頁面馬上以 `FZ_ERROR_LIMIT` 失敗，不必等 worker 30 秒的繪製時限；找不到未修正、也找不到修正後的寫法時，建置失敗：
    - `source/fitz/draw-device.c` 的 `fz_draw_end_tile`：1.28 會把在頁面上移動不到 1/16 像素的步距加倍，但格子大、步距小時，每重複一次仍要貼一次整格。進迴圈之前先估計要畫多少（像素 + 8 × 列數 + 64 × 重複次數；權重都低於最便宜的情況、也就是全透明格子量到的成本），超過 2^38 就拒絕。被拒絕的 pattern 在開發機上估計至少要畫 45 秒，本來就會撞到 30 秒的時限。反過來，沒被拒絕的仍可能畫很久，例如縮圖的小倍率與不透明的格子（#672）；
    - `source/pdf/pdf-op-run.c` 的 `pdf_show_pattern`：pattern 的資源裡有混合模式（或裝置要求不用 tile）時，MuPDF 每重複一次就把 pattern 的內容跑一次，這條路沒有任何步距的保護。重複超過 2^26 次就拒絕：內容是空的，在開發機上每次也要 1.4 µs，2^26 次至少 90 秒；
    - worker 把繪製時的 `FZ_ERROR_LIMIT` 回報成 `LimitExceeded`（`crates/pdf_worker/src/serve.rs` 的 `render_error_code`）；
  - `mupdf-sys/msbuild.rs` 連結 `advapi32`。
- **已移除的建置期修補**：OpenJPEG（#539）與 lcms2（#562）的修正已經在 1.28.6 內附的版本裡，建置期修補已經拿掉。
- **舊分支保留**：`v0.8.0-bundled-fixes`（`1989036c0019c1b7f23ccf075024e3997a66c1bc`）保留不動。
- 根目錄與 `fuzz/` 的 `Cargo.toml` 以 `[patch.crates-io]` 指向同一個 `rev`（`8520f108e2adc332ad53b7a61281137650275904`），`deny.toml` 的 `allow-git` 允許此 repository。
- **什麼時候可以拿掉 fork**：MuPDF 內附的 Tesseract 到 5.5.4 以上時，Tesseract 的修補與上面的檢查都拿掉；MuPDF 內附的 FreeType 到 2.15.0 以上時，FreeType 的修補拿掉；crates.io 上的 `mupdf`、`mupdf-sys` 也帶了這樣的 MuPDF 時，在升級的 PR 裡刪除兩個 `Cargo.toml` 的 `[patch.crates-io]` 與 `deny.toml` 的 `allow-git`，這一節改回 crates.io。
  - fork 也以 `FZ_ENABLE_MD=0` 關掉 Markdown 讀取器（#572）：crates.io 的 `mupdf-sys` 也要能關掉它，才能拿掉 fork，否則 `markdown_reader_is_compiled_out` 會失敗。
  - 文字擷取的修補（#628、#629、#638、#678）也要上游修好才能拿掉，否則 `crates/pdf_worker/tests/text.rs` 的 `text_that_is_drawn_and_clipped_comes_out_once`、`a_text_object_that_clips_and_then_draws_keeps_its_clip`、`a_clipped_glyph_that_stands_for_two_letters_gives_both` 與 `glyph_names_give_the_characters_they_stand_for` 會失敗。
  - tiling pattern 的上限（#550）也要上游有同樣的保護才能拿掉，否則 `crates/pdf_worker/tests/render_error.rs` 的 `tiling_patterns_that_cannot_finish_report_limit_exceeded_at_once` 會逾時失敗。
- **之後要升到更新的 1.28.x**：在分支加 commit 改 gitlink；檢查建置輸出（`<run>/output`）沒有 `Unable to patch`；Tesseract 與 FreeType 的差異和這裡寫的相同；照 #561 的規則更新 `bundled-libraries.md`，並由 Claude Code 私下對照已知問題；加上 `needs-security-review`。

## 編譯期功能

`crates/pdf_worker/Cargo.toml`：`default-features = false, features = ["base14-fonts", "bundled-fonts-droid", "epub", "html", "svg", "xps"]`（後四個是 B3-03 加的，見 [create-pdf.md](create-pdf.md)）。

| 功能 | 狀態 | 說明 |
|---|---|---|
| JavaScript（`js`） | **關閉** | 以 `FZ_ENABLE_JS=0` 編譯，MuPDF 的腳本支援只剩空殼。測試 `javascript_is_compiled_out` 在測試碼中刻意呼叫 `enable_js()`，確認 `is_js_supported()` 仍為 false。正式程式碼不得呼叫 `enable_js`（ADR 0001 的表單腳本沙盒日後另行評估） |
| Markdown（`FZ_ENABLE_MD`） | 關閉 | 1.28 新增，會依內容猜格式；worker 不需要；由 `markdown_reader_is_compiled_out` 確認 |
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
- release 的 `pdf_worker.exe` 因此包含它們：Tesseract 與 Leptonica 原始碼被編譯連結進 worker；
- worker 在 B2-10 實作 OCR 後，由 `crates/pdf_worker/src/ocr.rs` 宣告並呼叫 Tesseract C API 與 `fz_set_leptonica_mem`（ADR 0015）；MuPDF 內建的 `tessocr` 裝置與 PDF OCR 輸出器則未被呼叫；
- ADR 0015 採用的 OCR 就是使用這一份。

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
