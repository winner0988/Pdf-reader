# 內嵌第三方 C 函式庫清單（mupdf-sys）

對應工作卡 Refs #553。規格 §3「渲染引擎行程隔離」、[mupdf-binding.md](../architecture/mupdf-binding.md)、[attack-surface.md](attack-surface.md)。

`crates/pdf_worker` 透過 `mupdf-sys` 靜態編譯並連結 MuPDF 及其第三方 C/C++ 函式庫。由於 `cargo deny` 與 Dependabot 僅能追蹤 crates.io 上的 Rust 依賴，無法穿透檢測靜態內嵌的 C/C++ 原始碼，因此本文件維護這份清單，記錄各函式庫在目前所用 MuPDF 版本的來源版本、原始碼提交點、平臺納入情況與更新指引。

- **查驗基準版本**：MuPDF 1.27.2（git commit [`73d3100d46d8a9ad634f6ef035bbe78f0f947886`](https://github.com/ArtifexSoftware/mupdf/commit/73d3100d46d8a9ad634f6ef035bbe78f0f947886)）
- **mupdf-sys 來源**：[`winner0988/mupdf-rs`](https://github.com/winner0988/mupdf-rs) 的 `v0.8.0-openjpeg-read-sod` 分支（commit `31e8194dc379f53f1f6da2a0d6fd9ad759e76bc7`，見 [mupdf-binding.md](../architecture/mupdf-binding.md)）
- **查驗日期**：2026-11-14

---

## 函式庫清單

| 函式庫 | 用途 | 版本 | Git Commit | 查證依據（檔案與行號） | Windows 編入 | Linux 編入 | 備註 |
|---|---|---|---|---|---|---|---|
| **MuPDF** | 核心 PDF 與文件渲染引擎 | 1.27.2 | `73d3100d46d8a9ad634f6ef035bbe78f0f947886` | `include/mupdf/fitz/version.h#L26-L29` | 是 | 是 | 核心解析與渲染引擎；經由 `mupdf-sys` 靜態編譯連結。 |
| **FreeType** | 字型載入、解析與點陣化 | 2.13.3 | `42608f77f20749dd6ddc9e0536788eaad70ea4b5` | `thirdparty/freetype/include/freetype/freetype.h#L5175-L5177` | 是 | 是 | TrueType / OpenType 字型解碼與渲染。編譯時由 `slimftoptions.h` 關閉未使用的變形字型擴充。 |
| **HarfBuzz** | 複雜文字塑形（OpenType shaping） | 6.0.0 | `ff8031d14b30d7c1b10fa3466d5f7c458837647b` | `thirdparty/harfbuzz/src/hb-version.h#L44-L63` | 是 | 是 | 供文字排版塑形（`hb_shape`）。字型子集化未透過 HarfBuzz，由純 Rust `subsetter` crate 處理。 |
| **libjpeg (IJG)** | JPEG 影像解碼 | 9f | `7fe42a89507fac623fadf0c68fd1b10ebef66a23` | `thirdparty/libjpeg/jversion.h#L12`、`jpeglib.h#L40-L42` | 是 | 是 | 獨立 JPEG 群組（IJG）實作，供 DCT 濾鏡與點陣圖片解碼。 |
| **OpenJPEG** | JPEG 2000 影像解碼 | 2.5.4 | `5781f7478694c78e7a245dc21fe7fa6f65bf3b96` | `thirdparty/openjpeg/NEWS.md#L5`（標頭檔 `src/lib/openjp2/opj_config.h#L10-L12` 未更新仍標 2.5.3） | 是 | 是 | 供 JPXDecode 濾鏡解碼。建置時由 fork `winner0988/mupdf-rs`（commit `31e8194`）在 `mupdf-sys/build.rs` 的 `patch_mupdf_sources` 自動套用 OpenJPEG 上游修補（commit `91d08b11`，見附註與 [mupdf-binding.md](../architecture/mupdf-binding.md)）。 |
| **jbig2dec** | JBIG2 雙值圖片解碼 | 0.20 | `d915733ec69260c1fd2c458b06835cb57bb85ac7` | `thirdparty/jbig2dec/CHANGES#L1` | 是 | 是 | 供 JBIG2Decode 濾鏡使用，僅連結解碼函式庫，未包含命令列工具。 |
| **zlib** | Deflate / Inflate 串流壓縮與解壓縮 | 1.3.1 | `51b7f2abdade71cd9bb0e7a373ef2610ec6f9daf` | `thirdparty/zlib/zlib.h#L40` | 是 | 是 | 供 FlateDecode 濾鏡與串流解壓縮使用。僅調用串流 API，未包含高階檔案 I/O 工具。 |
| **lcms2** | 色彩管理（Little CMS 2） | 2.16 (lcms2mt) | `f75fad71d53efd58d7312bea21c2bedca0b9e6da` | `thirdparty/lcms2/configure.ac#L9`、`include/lcms2mt.h#L92` | 是 | 是 | 採用多執行緒分支（lcms2mt），用於 ICC 色彩設定檔轉換與色彩空間對齊。 |
| **gumbo-parser** | HTML5 解析器 | 0.10.1 | `61bd4e736c3b342e592f65be0d65ec6a0acd8dcb` | `thirdparty/gumbo-parser/configure.ac#L5`、`CHANGES.md#L1` | 是 | 是 | 供「從檔案建立 PDF」（`CreatePdf`）的 HTML 轉換解析使用。 |
| **Tesseract** | 光學字元辨識（OCR） | 5.5.0 | `4a25c1f7280654545557d077b96dc14026c45c1f` | `thirdparty/tesseract/VERSION#L1`（包裝標頭 `scripts/tesseract/tesseract/version.h#L28` 殘留 5.0.0-alpha） | 是 | 是（只在 Fuzz 的建置） | 供掃描頁 OCR 辨識（B2-10、ADR 0015）。由 `crates/pdf_worker/src/ocr.rs` 宣告並呼叫 `TessBaseAPICreate`、`TessBaseAPIInit5` 等 C API；輸入為 MuPDF 渲染之頁面點陣圖與使用者匯入之 `.traineddata`（由 `crates/ipc_contract/src/ocr.rs` 的 `check_language_data` 檢查）。Linux 僅於 Fuzz 建置中開啟（`fuzz/Cargo.toml` 啟用 `tesseract` feature）。 |
| **Leptonica** | 影像處理與分析庫（供 OCR 使用） | 1.85.0 | `0dc051249fa596016310e6d1509518fd34be4f63` | `thirdparty/leptonica/src/allheaders.h#L31-L33`、`CMakeLists.txt#L45` | 是 | 是（只在 Fuzz 的建置） | 與 Tesseract 搭配供 OCR 使用。MuPDF 以 `fz_set_leptonica_mem` 將其記憶體配置接管至 MuPDF context；輸入為記憶體點陣圖。Linux 僅在 Fuzz 建置中開啟。 |
| **Brotli** | 通用解壓縮演算法 | 1.1.0 | `6e092ab79683949f432d1c0cab88effa683c90d7` | `thirdparty/brotli/c/common/version.h#L20-L22` | 否 | 否 | `FZ_ENABLE_BROTLI=0` 編譯為空虛設函式（stub），未被參照的物件被 MSVC 連結器 `/OPT:REF` 剔除；Linux Makefile 透過 `make_bool("brotli", false)` 關閉。 |
| **MuJS** | 輕量級 JavaScript 直譯器 | 1.3.8 | `33c83d8553048fd7710a518821cdefa5818e1559` | `thirdparty/mujs/mujs.h#L10-L12` | 否 | 否 | `FZ_ENABLE_JS=0`（Linux `make_bool("mujs", false)`），不編譯／不連結執行引擎。由 `javascript_is_compiled_out` 測試驗證。 |
| **extract** | DOCX 輸出轉換工具 | 未標註版本（無獨立版本號） | `8750ac39c30a0d65119b426b5a491c5b8e8bf674` | `thirdparty/extract/`（目錄內無版本定義） | 否 | 否 | `FZ_ENABLE_DOCX_OUTPUT=0`，未啟用。Word 匯出由 `pdf_worker` 自行組裝 XML（見 [docx-export.md](../architecture/docx-export.md)）。 |
| **zxing-cpp** | 條碼與 QR Code 辨識 | 2.3.0 | `742aef959e12a41eda494df78fd567c2e715ff57` | `scripts/zxing-cpp/Version.h#L13-L17` | 否 | 否 | Windows 雖於 `libmupdf.vcxproj` 編譯 `barcode.c` 並關聯 `libmubarcode.vcxproj`，但 `pdf_worker` 未呼叫條碼函式且 `msbuild.rs` 只連結 `libmupdf` 與 `libthirdparty`，未連結進 `pdf_worker.exe`（發行二進位檔字串查驗為 0 筆匹配）；Linux 由 `build.rs:447` 設定 `make_bool("barcode", false)` 關閉。 |
| **zint** | 條碼繪製與編碼 | 2.13.0.9 (開發版) | `7a9fdd6cd00cd5bfd0082705d934c13ef84f25e1` | `thirdparty/zint/backend/zintconfig.h#L37-L40`、`backend/zint.h#L34` | 否 | 否 | 與 zxing-cpp 相同，未被 `pdf_worker` 呼叫且 `msbuild.rs` 未連結 barcode 專案庫，未連結進 `pdf_worker.exe`（發行二進位檔字串查驗為 0 筆匹配）；Linux 由 `make_bool("barcode", false)` 關閉。 |
| **curl** | 網路資料傳輸函式庫 | 7.66.0-DEV | `f43c6fbb2dbdf68dfdd0f0c3399e3711a621614a` | `thirdparty/curl/include/curl/curlver.h#L33` | 否 | 否 | Windows 未納入 `libmupdf.vcxproj`；Linux `HAVE_CURL=no`。`pdf_worker` 為零網路沙盒行程（ADR 0009）。 |
| **freeglut** | OpenGL 視窗與輸入工具庫 | 3.0.0 | `d5e2256d571b3ef66fb60716c99e35e9d3e570a2` | `thirdparty/freeglut/CMakeLists.txt#L20-L22` | 否 | 否 | Windows 未納入方案；Linux `HAVE_GLUT=no`。無 GUI / OpenGL 依賴。 |

### 附註：OpenJPEG 建置期修補

MuPDF 1.27.2 內附的 OpenJPEG（commit `5781f747`）存在 `opj_j2k_read_sod` 寫入 tile-part 索引前缺少範圍檢查的問題。專案使用 fork `winner0988/mupdf-rs`（commit `31e8194`），其在 `mupdf-sys/build.rs` 函式 `patch_mupdf_sources`（第 63 行，註解在第 210–213 行）於選擇 MSBuild 或 make 之前被呼叫，因此在 Windows 與 Linux 建置期皆會自動套用 OpenJPEG 上游 commit [`91d08b11`](https://github.com/uclouvain/openjpeg/commit/91d08b11a72764f6d199f32fa0c1b1abd4edc2ad) 的修復（詳見 [mupdf-binding.md](../architecture/mupdf-binding.md)）。

---

## 平臺編譯與連結行為說明

### Windows (MSVC)

1. **方案編譯**：Windows 上 `mupdf-sys` 透過 MSBuild 編譯 Visual Studio 方案（`platform/win32/mupdf.sln`）。
2. **前置處理器旗標**：編譯時以 `FZ_ENABLE_*` 控制功能（例如 `FZ_ENABLE_JS=0`、`FZ_ENABLE_BROTLI=0`、`FZ_ENABLE_DOCX_OUTPUT=0`）。關閉的功能在 MuPDF 內部實作為直接擲出 `FZ_ERROR_UNSUPPORTED` 的空函式 stub。
3. **未參照符號剔除**：MSVC 連結器預設開啟 `/OPT:REF` 與 `/OPT:ICF`。對於未被 `pdf_worker` 參照的函式庫物件（例如 Brotli、MuJS、extract 等），連結器不會將其代碼放入最終的 `pdf_worker.exe`。
4. **Tesseract 與 Leptonica 的編譯與呼叫**：Windows 方案預設包含 `libtesseract.vcxproj` 與 `liblept.vcxproj`，編譯時帶入 `HAVE_TESSERACT` 與 `HAVE_LEPTONICA`，靜態編譯連結進 `pdf_worker.exe`。`crates/pdf_worker/src/ocr.rs` 宣告並呼叫 `TessBaseAPICreate`、`TessBaseAPIInit5` 等 C API 執行掃描頁辨識（B2-10、ADR 0015），輸入為 MuPDF 渲染之頁面點陣圖與使用者匯入之 `.traineddata`（由主行程與 worker 的 `crates/ipc_contract/src/ocr.rs` `check_language_data` 檢查）。
5. **條碼函式庫未連結**：Windows 下 `platform/win32/libmupdf.vcxproj` 編譯 `source/fitz/barcode.c` 並將 `libmubarcode.vcxproj` 列為相依專案（第 1069、1424 行），且 `build.rs` 未定義 `FZ_ENABLE_BARCODE`（`config.h` 預設為 1）；但因 `pdf_worker` 未調用任何條碼 API，且 `mupdf-sys/msbuild.rs` 僅連結 `libmupdf` 與 `libthirdparty`（第 119–120 行），zxing-cpp 與 zint 的符號並未進入 `pdf_worker.exe`（發行二進位檔字串查驗：zxing 0、zint 0）。

### Linux (Make)

1. **Makefile 旗標**：`mupdf-sys/build.rs` 透過環境變數傳入 `HAVE_*` 與 `make_bool` 設定（如 `make_bool("brotli", false)`、`make_bool("mujs", false)`、`HAVE_CURL=no`、`HAVE_GLUT=no`）。Tesseract 則由 `mupdf-sys/make.rs:103` 依 cargo feature 設定 `USE_TESSERACT`（`make_bool("tesseract", tesseract)` 於 `build.rs:446`）。
2. **完全排除**：被設定為關閉的第三方函式庫，Linux Makefile 在編譯期完全不編譯其目錄下的 C/C++ 檔案，產生的 `libmupdf.a` 完全不包含其符號。
3. **Fuzz 建置納入 Tesseract**：Linux 發行版預設不編入 Tesseract 與 Leptonica；而在 Fuzz 建置中，`fuzz/Cargo.toml` 替 `mupdf-sys` 開啟了 `tesseract` feature，這會對整個 fuzz crate 生效（使 `mupdf-sys/make.rs` 設定 `USE_TESSERACT=yes` 編入 Tesseract 與 Leptonica），供 `fuzz_targets/ocr.rs` 等目標持續進行模糊測試。

---

## 清單查證與更新程序

當 MuPDF 或 `mupdf-sys` 升級版本時，負責人或開發者應遵循以下步驟驗證並更新本清單：

1. **取得最新原始碼**：
   在 `mupdf-sys` 的 checkout 目錄（例如本地或 `~/.cargo/git/checkouts/mupdf-rs-*/<hash>` 下的 MuPDF 原始碼目錄）執行：
   ```bash
   # 檢查 MuPDFFitz 主版本
   head -n 35 include/mupdf/fitz/version.h
   git rev-parse HEAD
   ```
2. **列出所有第三方子模組 commit**：
   在 MuPDF 原始碼目錄下執行：
   ```bash
   git ls-tree HEAD thirdparty/
   ```
   將輸出中的每個 submodule commit hash 與舊清單進行比對。
3. **比對各函式庫版本定義檔案**：
   針對有更新的子模組，依據上表「查證依據」列出的標頭檔或版本檔案路徑，檢視版本巨集或常數定義：
   - FreeType：`thirdparty/freetype/include/freetype/freetype.h`（搜尋 `FREETYPE_MAJOR`、`FREETYPE_MINOR`、`FREETYPE_PATCH`）
   - HarfBuzz：`thirdparty/harfbuzz/src/hb-version.h`（搜尋 `HB_VERSION_STRING`）
   - libjpeg：`thirdparty/libjpeg/jversion.h` 與 `jpeglib.h`
   - OpenJPEG：`thirdparty/openjpeg/NEWS.md`、`CHANGELOG.md`、`src/lib/openjp2/opj_config.h`
   - jbig2dec：`thirdparty/jbig2dec/CHANGES`
   - zlib：`thirdparty/zlib/zlib.h`（搜尋 `ZLIB_VERSION`）
   - lcms2：`thirdparty/lcms2/configure.ac`、`include/lcms2mt.h`（搜尋 `LCMS_VERSION`）
   - gumbo-parser：`thirdparty/gumbo-parser/configure.ac`、`CHANGES.md`
   - Tesseract：`thirdparty/tesseract/VERSION`
   - Leptonica：`thirdparty/leptonica/src/allheaders.h`
   - Brotli：`thirdparty/brotli/c/common/version.h`
   - MuJS：`thirdparty/mujs/mujs.h`
   - zxing-cpp：`scripts/zxing-cpp/Version.h`
   - zint：`thirdparty/zint/backend/zintconfig.h`
   - curl：`thirdparty/curl/include/curl/curlver.h`
   - freeglut：`thirdparty/freeglut/CMakeLists.txt`
4. **檢核建置旗標與啟用狀態**：
   - 檢查 `mupdf-sys/build.rs` 是否有新增或移除的 `make_bool` / `HAVE_*` 設定。
   - 檢查 `platform/win32/mupdf.sln` 與 `libmupdf.vcxproj` 是否有增減第三方專案或巨集定義。
   - 確認是否有新的建置期修補（例如 `build.rs` 內的 patch 邏輯）。
5. **檢視 worker 直接呼叫之 C 函式庫與 Fuzz features**：
   - 執行 `grep -rn 'extern "C"' crates/pdf_worker/src`，確認 worker 是否新增了直接調用之 C FFI 介面（例如 `ocr.rs` 宣告並調用 Tesseract C API）。
   - 檢查 `fuzz/Cargo.toml` 替 `mupdf-sys` 開啟了哪些 feature（例如 `features = ["tesseract"]` 會使 Linux fuzz 建置納入 Tesseract）。
6. **發行二進位檔字串查驗**：
   在發行建置的 `pdf_worker.exe` 中搜尋各函式庫關鍵字串（`grep -a -c <lib> <exe>`，例如 `grep -a -c zxing <exe>`、`grep -a -c zint <exe>`、`grep -a -c tesseract <exe>`），確認未啟用函式庫未被連結進產物，且啟用函式庫符合預期。
7. **更新清單與關聯文件**：
   - 更新本文件的表格內容與查驗日期。
   - 若有新增函式庫或版本變動，同步確認 [mupdf-binding.md](../architecture/mupdf-binding.md) 與 [attack-surface.md](attack-surface.md) 之說明。
