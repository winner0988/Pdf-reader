# 內嵌第三方 C 函式庫清單（mupdf-sys）

對應工作卡 Refs #553。規格 §3「渲染引擎行程隔離」、[mupdf-binding.md](../architecture/mupdf-binding.md)、[attack-surface.md](attack-surface.md)。

`crates/pdf_worker` 透過 `mupdf-sys` 靜態編譯並連結 MuPDF 及其第三方 C/C++ 函式庫。由於 `cargo deny` 與 Dependabot 僅能追蹤 crates.io 上的 Rust 依賴，無法穿透檢測靜態內嵌的 C/C++ 原始碼，因此本文件維護這份清單，記錄各函式庫在目前所用 MuPDF 版本的來源版本、原始碼提交點、平臺納入情況與更新指引。

- **查驗基準版本**：MuPDF 1.28.6（git commit [`da5f5eb890d7b3fb76903160b5f7f48ba1301492`](https://github.com/ArtifexSoftware/mupdf/commit/da5f5eb890d7b3fb76903160b5f7f48ba1301492)）
- **mupdf-sys 來源**：[`winner0988/mupdf-rs`](https://github.com/winner0988/mupdf-rs) 的 `v0.8.0-mupdf-1.28` 分支（commit `c183ec844106f29ed50a6db8bcf99714216e0513`，見 [mupdf-binding.md](../architecture/mupdf-binding.md)）
- **查驗日期**：2026-11-17

---

## 函式庫清單

| 函式庫 | 用途 | 版本 | Git Commit | 查證依據（檔案與行號） | Windows 編入 | Linux 編入 | 備註 |
|---|---|---|---|---|---|---|---|
| **MuPDF** | 核心 PDF 與文件渲染引擎 | 1.28.6 | `da5f5eb890d7b3fb76903160b5f7f48ba1301492` | `include/mupdf/fitz/version.h#L26-L29` | 是 | 是 | 核心解析與渲染引擎；經由 `mupdf-sys` 靜態編譯連結。 |
| **FreeType** | 字型載入、解析與點陣化 | 2.14.3 | `0a0221a1347e2f1e07c395263540026e9a0aa7c7` | `thirdparty/freetype/include/freetype/freetype.h#L5170-L5172` | 是 | 是 | TrueType / OpenType 字型解碼與渲染。編譯時由 `slimftoptions.h` 關閉未使用的變形字型擴充。 |
| **HarfBuzz** | 複雜文字塑形（OpenType shaping） | 13.0.1 | `c28ba35a1e6dc4b6b32e2b1d11fd2a408f6c86bb` | `thirdparty/harfbuzz/src/hb-version.h#L44-L63` | 是 | 是 | 供文字排版塑形（`hb_shape`）。字型子集化未透過 HarfBuzz，由純 Rust `subsetter` crate 處理。 |
| **libjpeg (IJG)** | JPEG 影像解碼 | 10 | `627faae398468d5d11e3d694dc325432f734b5c4` | `thirdparty/libjpeg/jversion.h#L12`、`jpeglib.h#L40-L42` | 是 | 是 | 獨立 JPEG 群組（IJG）實作，供 DCT 濾鏡與點陣圖片解碼。 |
| **OpenJPEG** | JPEG 2000 影像解碼 | 2.5.4 | `957029eb875eee1118743f200cb86da9d8289de2` | `thirdparty/openjpeg/NEWS.md#L5`（標頭檔 `src/lib/openjp2/opj_config.h#L10-L12` 未更新仍標 2.5.3） | 是 | 是 | 供 JPXDecode 濾鏡解碼。MuPDF 1.28.6 內附的版本已經有上游的修正，#572 拿掉了建置期修補（見附註與 [mupdf-binding.md](../architecture/mupdf-binding.md)）。 |
| **jbig2dec** | JBIG2 雙值圖片解碼 | 0.20 | `dc15c39bbbddc90f79c14563d2eb5a794106be8f` | `thirdparty/jbig2dec/CHANGES#L1` | 是 | 是 | 供 JBIG2Decode 濾鏡使用，僅連結解碼函式庫，未包含命令列工具。 |
| **zlib** | Deflate / Inflate 串流壓縮與解壓縮 | 1.3.2 | `da607da739fa6047df13e66a2af6b8bec7c2a498` | `thirdparty/zlib/zlib.h#L44` | 是 | 是 | 供 FlateDecode 濾鏡與串流解壓縮使用。僅調用串流 API，未包含高階檔案 I/O 工具。 |
| **lcms2** | 色彩管理（Little CMS 2） | 2.19 (lcms2mt) | `d69c64417c4a33a4629957fc0e08f4f4c5abcc3d` | `thirdparty/lcms2/configure.ac#L10`、`include/lcms2mt.h#L102` | 是 | 是 | 採用多執行緒分支（lcms2mt），用於 ICC 色彩設定檔轉換與色彩空間對齊。MuPDF 1.28.6 內附的版本已經有上游的修正，#572 拿掉了建置期修補（見附註與 [mupdf-binding.md](../architecture/mupdf-binding.md)）。 |
| **gumbo-parser** | HTML5 解析器 | 0.10.1 | `61bd4e736c3b342e592f65be0d65ec6a0acd8dcb` | `thirdparty/gumbo-parser/configure.ac#L5`、`CHANGES.md#L1` | 是 | 是 | 供「從檔案建立 PDF」（`CreatePdf`）的 HTML 轉換解析使用。 |
| **Tesseract** | 光學字元辨識（OCR） | 5.5.2 | `ad0f76d971946512a62c933f490c8111877eddaa` | `thirdparty/tesseract/VERSION#L1`（包裝標頭 `scripts/tesseract/tesseract/version.h#L28` 殘留 5.0.0-alpha） | 是 | 是（只在 Fuzz 的建置） | 供掃描頁 OCR 辨識（B2-10、ADR 0015）。建置時由 fork `winner0988/mupdf-rs`（commit `c183ec84`）在 `mupdf-sys/build.rs` 的 `patch_mupdf_sources` 自動套用 Tesseract 上游修補（#563 的 5 個 commit 與 #577 套的上游 5.5.3、5.5.4 讀取語言資料檢查 22 個 commit，共 25 個檔案 +464 −55；只改到舊引擎與 `FAST_FLOAT` 的不套，建置時會檢查這兩個設定，見附註與 [mupdf-binding.md](../architecture/mupdf-binding.md)）。由 `crates/pdf_worker/src/ocr.rs` 宣告並呼叫 `TessBaseAPICreate`、`TessBaseAPIInit5` 等 C API；輸入為 MuPDF 渲染之頁面點陣圖與使用者匯入之 `.traineddata`（由 `crates/ipc_contract/src/ocr.rs` 的 `check_language_data` 檢查）。Linux 僅於 Fuzz 建置中開啟（`fuzz/Cargo.toml` 啟用 `tesseract` feature）。修補仍然保留。 |
| **Leptonica** | 影像處理與分析庫（供 OCR 使用） | 1.87.0 | `7fbe9ea76d1e0c48a02b2ed371d24f92c7bac40c` | `thirdparty/leptonica/src/allheaders.h#L31-L33` | 是 | 是（只在 Fuzz 的建置） | 與 Tesseract 搭配供 OCR 使用。MuPDF 以 `fz_set_leptonica_mem` 將其記憶體配置接管至 MuPDF context；輸入為記憶體點陣圖。Linux 僅在 Fuzz 建置中開啟。 |
| **Brotli** | 通用解壓縮演算法 | 1.2.1 | `05d2263409056fde56805e9cdabd2d31a1f1d427` | `thirdparty/brotli/c/common/version.h#L20-L22` | 否 | 否 | `FZ_ENABLE_BROTLI=0` 編譯為空虛設函式（stub），未被參照的物件被 MSVC 連結器 `/OPT:REF` 剔除；Linux Makefile 透過 `make_bool("brotli", false)` 關閉。 |
| **MuJS** | 輕量級 JavaScript 直譯器 | 1.3.8 | `2c61a7bed7e7625c03c6e82e644a711ac4471c1e` | `thirdparty/mujs/mujs.h#L10-L12` | 否 | 否 | `FZ_ENABLE_JS=0`（Linux `make_bool("mujs", false)`），不編譯／不連結執行引擎。由 `javascript_is_compiled_out` 測試驗證。 |
| **extract** | DOCX 輸出轉換工具 | 未標註版本（無獨立版本號） | `8750ac39c30a0d65119b426b5a491c5b8e8bf674` | `thirdparty/extract/`（目錄內無版本定義） | 否 | 否 | `FZ_ENABLE_DOCX_OUTPUT=0`，未啟用。Word 匯出由 `pdf_worker` 自行組裝 XML（見 [docx-export.md](../architecture/docx-export.md)）。 |
| **cmark-gfm** | Markdown 解析器（GitHub Flavored Markdown） | 0.29.0.gfm.13 | `74e0f5b6b62bbdd1e262c92f4bb2c6c871c6307e` | `thirdparty/cmark-gfm/changelog.txt#L1`、`CMakeLists.txt#L4-L8` | 否 | 否 | 供 MuPDF 1.28 新增之 Markdown 讀取器（`FZ_ENABLE_MD`）使用。`mupdf-sys/build.rs` 設定 `FZ_ENABLE_MD=0` 關閉此讀取器，沒有任何程式呼叫它；Windows 雖於 `libthirdparty.vcxproj` 編譯原始碼，但因未被參照由 MSVC 連結器 `/OPT:REF` 剔除，未進入 `pdf_worker.exe`；Linux 在 `html` 開啟時 `Makerules` 會設 `USE_CMARK_GFM := yes` 編譯 cmark-gfm，未進執行檔的理由和 Windows 相同，為 `FZ_ENABLE_MD=0` 之後沒有程式參照它（Linux 只在 CI 上建置，執行檔未實際查驗字串）。由 `markdown_reader_is_compiled_out` 測試驗證。 |
| **zxing-cpp** | 條碼與 QR Code 辨識 | 2.3.0 | `742aef959e12a41eda494df78fd567c2e715ff57` | `scripts/zxing-cpp/Version.h#L13-L17` | 否 | 否 | Windows 雖於 `libmupdf.vcxproj` 透過 `LinkLibraryDependencies` 將 `libmubarcode` 物件併入 `libmupdf.lib`，但靜態函式庫僅拉入有被參照之物件，因 `pdf_worker` 未調用任何條碼函式，故未連結進 `pdf_worker.exe`（發行二進位檔字串查驗為 0 筆匹配）；Linux 由 `build.rs:447` 設定 `make_bool("barcode", false)` 關閉。 |
| **zint** | 條碼繪製與編碼 | 2.13.0.9 (開發版) | `7a9fdd6cd00cd5bfd0082705d934c13ef84f25e1` | `thirdparty/zint/backend/zintconfig.h#L37-L40`、`backend/zint.h#L34` | 否 | 否 | 與 zxing-cpp 相同，條碼物件雖併入 `libmupdf.lib`，但因未被 `pdf_worker` 調用，靜態函式庫物件未被拉入，未連結進 `pdf_worker.exe`（發行二進位檔字串查驗為 0 筆匹配）；Linux 由 `make_bool("barcode", false)` 關閉。 |
| **curl** | 網路資料傳輸函式庫 | 7.66.0-DEV | `f43c6fbb2dbdf68dfdd0f0c3399e3711a621614a` | `thirdparty/curl/include/curl/curlver.h#L33` | 否 | 否 | Windows 未納入 `libmupdf.vcxproj`；Linux `HAVE_CURL=no`。`pdf_worker` 為零網路沙盒行程（ADR 0009）。 |
| **freeglut** | OpenGL 視窗與輸入工具庫 | 3.0.0 | `d5e2256d571b3ef66fb60716c99e35e9d3e570a2` | `thirdparty/freeglut/CMakeLists.txt#L20-L22` | 否 | 否 | Windows 未納入方案；Linux `HAVE_GLUT=no`。無 GUI / OpenGL 依賴。 |

### 附註：OpenJPEG 建置期修補

MuPDF 1.28.6 內附的版本已經有上游的修正，#572 拿掉了建置期修補。

### 附註：lcms2 建置期修補

MuPDF 1.28.6 內附的版本已經有上游的修正，#572 拿掉了建置期修補。

### 附註：Tesseract 建置期修補

MuPDF 1.28.6 內附的 Tesseract（基準為 5.5.2，commit `ad0f76d971946512a62c933f490c8111877eddaa`）缺少對 traineddata 讀取時的計數與長度邊界檢查，以及 LSTM 閘矩陣與 recoder 的合理性檢查。專案使用 fork `winner0988/mupdf-rs`（commit `c183ec844106f29ed50a6db8bcf99714216e0513`），其在 `mupdf-sys/build.rs` 的 `patch_mupdf_sources` 於呼叫 MSBuild 或 make 之前被呼叫，因此在 Windows 與 Linux 建置期皆會自動套用 Tesseract 上游修補：包括 #563 的五個 commit（`82727cc1`、`55287a94`、`103dc134`、`b494ac18` 與 `c94a5532`，`103dc134` 加的 `WeightMatrix::Dim1()`／`Dim2()` 是 `b494ac18` 要用的），加上 #577 套的上游 5.5.3、5.5.4 讀取語言資料的檢查（22 個 commit，清單在 fork 的 `mupdf-sys/build.rs`；共 25 個檔案 +464 −55）。只改到舊引擎（`DISABLED_LEGACY_ENGINE`）與 `FAST_FLOAT` 的不套，建置時會檢查這兩個設定（原始碼裡既找不到未修正、也找不到修正後的寫法時，或 MuPDF 開始編譯舊引擎／`FAST_FLOAT` 時，建置皆會失敗；詳見 [mupdf-binding.md](../architecture/mupdf-binding.md)）。修補仍然保留。

---

## 平臺編譯與連結行為說明

### Windows (MSVC)

1. **方案編譯**：Windows 上 `mupdf-sys` 透過 MSBuild 編譯 Visual Studio 方案（`platform/win32/mupdf.sln`）。
2. **前置處理器旗標**：編譯時以 `FZ_ENABLE_*` 控制功能（例如 `FZ_ENABLE_JS=0`、`FZ_ENABLE_BROTLI=0`、`FZ_ENABLE_DOCX_OUTPUT=0`）。關閉的功能在 MuPDF 內部實作為直接擲出 `FZ_ERROR_UNSUPPORTED` 的空函式 stub。
3. **未參照物件與符號剔除**：連結靜態函式庫時，未被參照的物件（`.obj`，例如 Brotli、MuJS、extract、barcode 等未被調用的函式庫物件）本來就不會被連結器拉入最終二進位檔；MSVC 連結器預設開啟之 `/OPT:REF` 與 `/OPT:ICF`，則進一步將已拉入物件中未被參照的函式與資料（COMDAT）自最終產物剔除。
4. **Tesseract 與 Leptonica 的編譯與呼叫**：Windows 方案預設包含 `libtesseract.vcxproj` 與 `liblept.vcxproj`，編譯時帶入 `HAVE_TESSERACT` 與 `HAVE_LEPTONICA`，靜態編譯連結進 `pdf_worker.exe`。`crates/pdf_worker/src/ocr.rs` 宣告並呼叫 `TessBaseAPICreate`、`TessBaseAPIInit5` 等 C API 執行掃描頁辨識（B2-10、ADR 0015），輸入為 MuPDF 渲染之頁面點陣圖與使用者匯入之 `.traineddata`（由主行程與 worker 的 `crates/ipc_contract/src/ocr.rs` `check_language_data` 檢查）。
5. **條碼函式庫未連結**：Windows 下 `platform/win32/libmupdf.vcxproj` 編譯 `source/fitz/barcode.c`（第 1069 行）並將 `libmubarcode.vcxproj` 列為相依專案（第 1424 行），且因 `<ProjectReference>` 設定 `<LinkLibraryDependencies>true</LinkLibraryDependencies>`（第 477 行起），條碼專案的物件會被併入 `libmupdf.lib`；`mupdf-sys/msbuild.rs`（第 119–120 行）亦連結 `libmupdf` 與 `libthirdparty`。然而連結器只從靜態函式庫拉入有被參照的物件，由於 `pdf_worker` 未調用任何條碼 API（不同於 `crates/pdf_worker/src/ocr.rs` 呼叫 Tesseract C API 使其物件被拉入），因此 zxing-cpp 與 zint 的物件不會被拉進最終的 `pdf_worker.exe`（發行二進位檔字串查驗：zxing 0、zint 0）。

### Linux (Make)

1. **Makefile 旗標**：`mupdf-sys/build.rs` 透過環境變數傳入 `HAVE_*` 與 `make_bool` 設定（如 `make_bool("brotli", false)`、`make_bool("mujs", false)`、`HAVE_CURL=no`、`HAVE_GLUT=no`）。Tesseract 則由 `mupdf-sys/make.rs:103` 依 cargo feature 設定 `USE_TESSERACT`（`make_bool("tesseract", tesseract)` 於 `build.rs:446`）。
2. **完全排除**：被設定為關閉的第三方函式庫，Linux Makefile 在編譯期完全不編譯其目錄下的 C/C++ 檔案，產生的 `libmupdf.a` 完全不包含其符號。
3. **Fuzz 建置納入 Tesseract**：專案沒有 Linux 發行版，Linux 只在 CI 上建置（例如 Fuzz），一般建置沒有開 `tesseract` feature，預設不編入 Tesseract 與 Leptonica；而在 Fuzz 建置中，`fuzz/Cargo.toml` 替 `mupdf-sys` 開啟了 `tesseract` feature，這會對整個 fuzz crate 生效（使 `mupdf-sys/make.rs` 設定 `USE_TESSERACT=yes` 編入 Tesseract 與 Leptonica），供 `fuzz_targets/ocr.rs` 等目標持續進行模糊測試。

---

## 清單查證與更新程序

本程序適用於兩種時機：每次發行 minor 或 major 版本前的例行對照（見 [release.md](../release.md#發行前) 的發行前檢查表），以及升級 MuPDF 或變更 `mupdf-sys` 來源時的重查與驗證（見 [mupdf-binding.md](../architecture/mupdf-binding.md#升級與來源變更規則)）。負責人或開發者應遵循以下步驟驗證並更新本清單：

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
   - 將對照結果依 [SECURITY.md](../../SECURITY.md) 私下回報負責人（不公開於 PR、CHANGELOG 或 commit）。
   - 若有新增函式庫或版本變動，同步確認 [mupdf-binding.md](../architecture/mupdf-binding.md#升級與來源變更規則) 與 [attack-surface.md](attack-surface.md) 之說明。
