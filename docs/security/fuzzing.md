# Fuzzing

對應工作卡 QA-03；ADR 0008：隔離能限制損害，但不能取代找出問題。

## 目標

| 目標（`fuzz/fuzz_targets/`） | 測什麼 | 種子 |
|---|---|---|
| `worker_messages` | IPC 的兩端：主行程解碼並驗證 worker 的回應（`decode::<WorkerResponse>`、`check_hello`、`Validate`），worker 解碼主行程的請求，以及長度前綴的分框（`read_frame`） | `fuzz_seeds` 為每一種訊息寫一個有效的 payload |
| `open_document` | 以 MuPDF 開啟 PDF，再做 worker 會做的每件事：頁面尺寸、渲染（0.1 倍、旋轉 90°）、連結、搜尋（前兩頁）、Word 文件（前兩頁，B3-02）、目錄、主動內容掃描 | `tests/corpus/` 的 PDF（QA-01，不含隨需產生的大型檔） |
| `read_document` | 以 MuPDF 開啟 PDF（支援密碼開啟：試 `user`、`owner` 與錯誤密碼），讀取前兩頁的註解（`page_annotations`）、文字方塊（`text_box`）、表單欄位（`page_fields`）、頁面文字（`page_text`），以及文件層級的註解摘要（`annotation_summary`）、個資搜尋（`find_personal_data`）、遮蔽摘要（`redaction_summary`）、附件清單與解碼（`list_attachments`、`write_attachment`，上限 1 MiB）、簽章欄位（`signature_fields`，讀取 `/Contents`）（#426，由 `open_document` 拆出） | 同 `open_document`（複製過去） |
| `text_box_font` | 字型檔（B2-31，[text-box-engine.md](../architecture/text-box-engine.md)）：先以 `inspect_font` 檢查字型檔的位元組並列出字體（B2-32，`InspectFont`）；再以 `skrifa` 讀取、以 `subsetter` 裁成文字用到的字形，做成文字方塊的模型。做得出模型的字型再放進一頁、渲染（MuPDF 讀取裁出來的字型）、存檔、重新開啟、渲染與搜尋。每個輸入試第一個字型與字型集合的第二個字型，文字依輸入的長度在英文、帶重音的拉丁字母、中文、日文四組中選一組（不佔用字型的位元組） | 隨 worker 附帶的 Droid Sans Fallback，加上 runner 上的 TrueType 字型（至多 20 個、各在 2 MB 內）；這個目標的輸入上限是 5 MiB，其他目標是 1 MiB |
| `compress_images` | 以 MuPDF 開啟 PDF，寫出壓縮圖片的副本（B3-01）：MuPDF 解碼每張圖片、縮小、編碼成 JPEG、換回去、完整重寫（品質低、尺寸上限 256、時間 2 秒） | 同 `open_document`（複製過去） |
| `create_pdf` | 「從檔案建立 PDF」（B3-03）：第一個位元組決定種類（圖片、網頁、純文字、EPUB、XPS、SVG、Word、Excel、PowerPoint），其餘是檔案；`PdfBuilder` 讀取、轉換、寫出（圖片會加兩次，試多頁的路徑） | `tests/corpus/sources/` 與 `tests/corpus/images/`，前面加種類的編號（CI 的 `Seed corpus` 步驟）；包含謊稱 3.5 GiB 的 Word 檔 |
| `exif_clean` | 以 MuPDF 開啟 PDF，寫出清除圖片 EXIF 的副本（B3-04，批次處理）：每個 JPEG 串流（包括藏在無損濾鏡後面的）讀出來、切掉不是畫面的區段、換回去、完整重寫 | 同 `open_document`（複製過去），其中 `benign/photo-exif.pdf` 有相機與程式寫進去的各種資訊 |
| `stamp_image` | 自訂圖章與圖片浮水印的圖片（B2-08、B3-04，`PrepareStampImage`）：以 `prepare_stamp_picture` 解碼 PNG 或 JPEG（檢查檔頭與尺寸、MuPDF 解碼、縮小並重新編碼為只有像素的 PNG），成功時放上一頁作為圖章註解並渲染（0.5 倍） | `tests/corpus/images/`（PNG 與 JPEG 圖片） |
| `language_data` | 使用者匯入與 worker 接收的 Tesseract 語言資料（`.traineddata`）檔頭檢查（`check_language_data`，#429）：目錄項目數（23–64）、位移不可倒退或超出檔案、必須包含 LSTM 模型所需項目 | `fuzz_seeds` 產生的有效目錄樣本，加上安裝程式內附的 `eng.traineddata` 前 1 MiB（`head -c 1048576`） |
| `ocr` | 灰階圖片與掃描頁的文字辨識（ADR 0015，#486）：第一個位元組決定種類（0 為圖片，1 為掃描頁）。(a) 圖片依前 6 個位元組的寬、高、ppi，以內附的 `eng` 資料辨識（寬高上限 12,000、像素上限 400 萬、ppi 1–200，逾時 500ms）；(b) 掃描頁開啟 PDF，對第 1 頁檢查 `ocr_is_scan`，若是掃描頁則渲染成灰階（上限 400 萬像素）、以 `eng` 資料辨識（逾時 500ms）並建立文字層（`page_text`）。兩者共用行程內唯一的 `Recogniser`。不把改壞的語言模型交給 Tesseract，因為 Tesseract 讀不了模型時以 `ASSERT_HOST` 結束行程（`abort`），fuzz 走不過去；模型的檔頭由 `language_data` 涵蓋，模型本身沒有 fuzz | `fuzz/seed_ocr.mjs` 產生的圖片樣本（前綴 00）、`tests/corpus/benign/scanned-text.pdf` 與 `image-only.pdf`（前綴 01） |
| `edit_document` | 對不可信文件套用 1～4 個編輯（#427，涵蓋 `WorkerEdit` 29 種中的 26 種，#478 加的 `AddShape`、`SetLineEnds`、`SetShapeStyle` 還沒有：`RotatePages`、`DeletePages`、`MovePages`、`InsertBlankPage`、`InsertPages`、`AddHighlight`〔含底線、刪除線、波浪線〕、`AddNote`、`DeleteAnnotation`、`SetHighlightColor`、`SetNoteText`、`SetFieldValue`、`FlattenForm`、`AddRedaction`、`AddRedactionGroup`、`ApplyRedactions`、`AddInk`、`AddStamp`、`SetAnnotationRect`、`SetPictureAngle`、`AddImageStamp`、`SetTextBox`、`PlaceTextBox`、`AddBookmark`、`RenameBookmark`、`DeleteBookmark`、`MoveBookmark`；參數經 `Validate` 檢查）、存檔（完整重寫；已簽章文件以增量更新）、重新開啟並渲染第一頁，並執行副動作（`SavePages`、`Revert`、`PrepareSource` 再 `InsertPages`，或 `PrivacyCopy`、`SanitizedCopy`、`EncryptedCopy`、`WatermarkCopy`、`PageMarksCopy`、`WriteIdentifier`、`PrepareSignature` 等 7 種副本） | 同 `open_document`，檔案前面加上 32 個位元組的動作與參數前綴（CI 的 `Seed corpus` 步驟輪流配置副動作與編輯種類） |

- 對所有輸入，程式都必須正常接受或拒絕：不能 panic、不能無限制地配置記憶體、不能出現記憶體錯誤。
- 任何一種都算崩潰，libFuzzer 會把觸發它的輸入存到 `fuzz/artifacts/<目標>/`。
- **字型目標與 `subsetter` 的溢位檢查**：cargo-fuzz 預設開啟溢位檢查，出貨的 release 建置沒有。裁字型的 `subsetter` 0.2.6 在 `name.rs` 對兩個 `u16` 相加時不檢查（上游 [typst/subsetter#30](https://github.com/typst/subsetter/issues/30)）：出貨的建置裡總和繞回去，結果是這筆名稱取不到、字型被拒絕；開著檢查就 panic，fuzz 會在第一個壞掉的 `name` 表就停下來，其他什麼都看不到。所以 `fuzz/Cargo.toml` 只對 `subsetter` 關掉溢位檢查（`[profile.release.package.subsetter]`），我們自己的程式碼仍然開著。上游修好、`subsetter` 升級後，就可以拿掉這一段。

## 自動執行（`.github/workflows/fuzz.yml`）

- **排程**：每週一，Linux runner，每個目標 20 分鐘。
- **PR**：修改 `fuzz/`、`crates/ipc_contract/`、`crates/pdf_worker/`、`tests/corpus/` 或這個 workflow 時，每個目標跑 2 分鐘。
- **手動**：Actions → Fuzz → Run workflow。
- **MuPDF 也被偵測**：
  - workflow 以 `CC=clang`、`CFLAGS=-fsanitize=fuzzer-no-link,address` 建置，MuPDF 與它的 C 函式庫也有覆蓋率回饋與 AddressSanitizer，不只 Rust 程式碼。
  - 只設 C 的旗標，因為 libFuzzer 本身是 C++，不能對自己插樁。
- **Linux fuzz 建置包含 Tesseract**：
  - Windows 上 `mupdf-sys` 0.8.0 透過 MSBuild 建置時無論 feature 為何都會包含 Tesseract，因此 app 的 OCR 正常運作。
  - Linux 上以 make 建置時會遵循 Cargo feature（`mupdf-sys` 的 `build.rs` 的 `make_bool("tesseract", …)`）。`crates/pdf_worker/Cargo.toml` 未開啟 `tesseract`，因此只在 `fuzz/Cargo.toml` 顯式開啟 `mupdf-sys` 的 `tesseract` feature，由 Cargo 的 feature 合併機制在 Linux fuzz 建置中啟用 Tesseract 與 Leptonica，不改動 worker 的正式建置設定。
- **摘要**：每個目標在 job 摘要列出執行次數、每秒次數、覆蓋率（edges）、features、語料數量與大小、記憶體高峰。
- **崩潰時**：job 失敗。樣本與完整日誌**加密後**才上傳為 `fuzz-crash-<目標>` artifact，保留 7 天；見下方「公開 repo 的保護」。
- **權限**：只有 `contents: read`。
- **一般 CI**：`Rust (Windows)` 以 `cargo check` 確認 fuzz 目標可以編譯；實際的 libFuzzer 建置需要 nightly，只在這個 workflow 中進行。

### 公開 repo 的保護

repo 是公開的：Actions 的日誌任何人都能看，artifact 只要登入 GitHub 就能下載。崩潰樣本可能就是可利用的攻擊檔案，所以 workflow 做了以下保護（負責人決定，[#57](https://github.com/winner0988/Pdf-reader/issues/57)）：

- **日誌只有統計**：
  - libFuzzer 的輸出（AddressSanitizer 報告、panic 訊息，以及它以 Base64 印出的小型輸入）只寫進 runner 上的 `fuzz.log`，不會出現在日誌中。
  - 日誌與 job 摘要只顯示執行次數、覆蓋率等統計，以及「找到崩潰」這件事本身。
  - fuzz 目標另外建置，所以編譯錯誤仍然看得到。
- **樣本加密**：
  - 崩潰時，`fuzz/artifacts/` 與 `fuzz.log` 以 [age](https://age-encryption.org/) 加密給 `.github/fuzz-recipient.txt` 中的公鑰，只有持有私鑰的負責人能解開。
  - age 從 Ubuntu 的套件庫安裝，而且只在失敗時安裝。
- **沒有公鑰就不上傳**：
  - `.github/fuzz-recipient.txt` 沒有 `age1…` 公鑰時，workflow 只發出警告，不上傳任何東西。
  - 這時要在本機重現（見「在本機執行」）。
  - 負責人的公鑰在 2026-11-04 才加入（#456）。在那之前找到的崩潰都沒有留下樣本，例如 2026-10-19 的 `text_box_font`，以及 10-26、11-02 的 `create_pdf`。

#### 負責人的金鑰（只需做一次）

1. 安裝 age：`winget install FiloSottile.age`。
2. 在 repo 以外的位置產生金鑰（PowerShell）：

   ```powershell
   New-Item -ItemType Directory -Force "$env:USERPROFILE\.age" | Out-Null
   age-keygen -o "$env:USERPROFILE\.age\pdf-reader-fuzz.txt"
   ```

3. 把它印出的 `Public key: age1…` 中的 `age1…` 放進 `.github/fuzz-recipient.txt`，以 PR 提交。公鑰可以公開。**私鑰檔不得提交或傳給任何人**，請另外備份。
4. 確認兩者相符：下面這個指令從私鑰檔算出公鑰，印出來的要和 `.github/fuzz-recipient.txt` 裡的那一行相同。

   ```powershell
   age-keygen -y "$env:USERPROFILE\.age\pdf-reader-fuzz.txt"
   ```

## 在本機執行

需要 nightly Rust 與 cargo-fuzz（都來自 Rust 官方來源）：

```bash
rustup toolchain install nightly --profile minimal
```

```bash
cargo install cargo-fuzz --version 0.13.2 --locked
```

準備種子（`fuzz/corpus/` 與 `fuzz/artifacts/` 不提交）：

```bash
cargo run --locked -p ipc_contract --bin fuzz_seeds -- fuzz/corpus/worker_messages
```

```bash
mkdir -p fuzz/corpus/open_document && find tests/corpus -name '*.pdf' -not -path '*/large/*' -exec cp {} fuzz/corpus/open_document/ \;
```

```bash
mkdir -p fuzz/corpus/stamp_image && cp tests/corpus/images/* fuzz/corpus/stamp_image/
```

`create_pdf` 的種子：每個檔案前面加一個位元組，是種類的編號（圖片 0、網頁 1、純文字 2、EPUB 3、XPS 4、SVG 5、Word 6、Excel 7、PowerPoint 8；順序同 `fuzz/fuzz_targets/create_pdf.rs` 的 `KINDS`）。做法見 `.github/workflows/fuzz.yml` 的 `Seed corpus` 步驟；在本機（Git Bash）：

```bash
mkdir -p fuzz/corpus/create_pdf
seed() { printf "\x$1" | cat - "$2" > "fuzz/corpus/create_pdf/$(basename "$2")"; }
for file in tests/corpus/images/*; do seed 00 "$file"; done
seed 01 tests/corpus/sources/web-page.html; seed 02 tests/corpus/sources/text.txt; seed 03 tests/corpus/sources/book.epub
seed 04 tests/corpus/sources/page.xps; seed 05 tests/corpus/sources/picture.svg; seed 06 tests/corpus/sources/letter.docx
seed 06 tests/corpus/sources/bomb-declared-size.docx; seed 07 tests/corpus/sources/sheet.xlsx; seed 08 tests/corpus/sources/slides.pptx
```

執行（例如 5 分鐘）：

```bash
cargo +nightly fuzz run open_document fuzz/corpus/open_document -- -max_total_time=300 -max_len=1048576 -rss_limit_mb=4096
```

字型的目標用 worker 附帶的字型當種子（建置過一次，原始碼就在 cargo 的 registry 裡），輸入上限 5 MiB：

```bash
mkdir -p fuzz/corpus/text_box_font && cp "$HOME"/.cargo/registry/src/*/mupdf-fonts-droid-*/fonts/*.ttf fuzz/corpus/text_box_font/
```

```bash
cargo +nightly fuzz run text_box_font fuzz/corpus/text_box_font -- -max_total_time=300 -max_len=5242880 -rss_limit_mb=4096
```

`language_data` 的種子：`fuzz_seeds` 產生的目錄樣本，加上安裝程式內附的英文語言檔前 1 MiB：

```bash
mkdir -p fuzz/corpus/language_data
cargo run --locked -p ipc_contract --bin fuzz_seeds -- fuzz/corpus/language_data language_data
head -c 1048576 src-tauri/resources/tessdata/eng.traineddata > fuzz/corpus/language_data/eng_1m.traineddata
```

```bash
cargo +nightly fuzz run language_data fuzz/corpus/language_data -- -max_total_time=300 -max_len=1048576 -rss_limit_mb=4096
```

`ocr` 的種子：`fuzz/seed_ocr.mjs` 產生的圖片樣本，加上語料庫中的掃描頁 PDF：

```bash
mkdir -p fuzz/corpus/ocr
node fuzz/seed_ocr.mjs fuzz/corpus/ocr
seed_ocr() { printf "\x$1" | cat - "$2" > "fuzz/corpus/ocr/$(basename "$2")"; }
seed_ocr 01 tests/corpus/benign/scanned-text.pdf
seed_ocr 01 tests/corpus/benign/image-only.pdf
```

```bash
cargo +nightly fuzz run ocr fuzz/corpus/ocr -- -max_total_time=300 -max_len=1048576 -rss_limit_mb=4096
```

### Windows

- **ASan 執行階段**：
  - Rust 的 AddressSanitizer 需要與 nightly 的 LLVM 相容的執行階段。Visual Studio 2019 內附的版本太舊，會以 `STATUS_DLL_INIT_FAILED` 結束。
  - 已安裝的 LLVM（見 [mupdf-binding.md](../architecture/mupdf-binding.md)）可以用：把它的執行階段目錄放到 `PATH` 前面，並讓連結器使用它的匯入程式庫。
  - 路徑中的空白要用 8.3 短名稱。
- **限制**：
  - `--sanitizer none` 在 Windows 無法連結（MSVC 連結器沒有 libFuzzer 需要的 section 符號）。
  - Windows 上的 MuPDF 由 MSBuild 建置，不會插樁，只有 Rust 程式碼有覆蓋率回饋。要測 MuPDF 本身，請看 Linux 的排程結果。

在 Git Bash 中（LLVM 版本目錄依實際安裝調整）：

```bash
export PATH="/c/Program Files/LLVM/lib/clang/23/lib/windows:$PATH"
export RUSTFLAGS="-Lnative=C:/PROGRA~1/LLVM/lib/clang/23/lib/windows"
cargo +nightly fuzz run worker_messages fuzz/corpus/worker_messages -- -max_total_time=60
```

開發時的本機結果（Windows 11，2026-09-24）：

- `worker_messages`：31 秒 186 萬次；
- `open_document`：91 秒 20,938 次（每秒約 230 次）；
- `create_pdf`（B3-03，2026-10-14）：241 秒 12,507 次（每秒約 52 次，每次都真的產生 PDF），沒有崩潰；
- `ocr`（#431，CI Linux smoke run）：40 次（每秒約 10 次，覆蓋率 edges 15,596、features 18,987，peak memory 3793 MB，找到崩潰，照公開 repo 的保護私下處理）；
- 兩者都沒有崩潰。

CI 結果（Linux runner，2026-11-04，#426）：

- `open_document`（之前的每秒次數，run 41868170550）：120 秒 7,970 次（每秒 65 次）；加讀取後掉到每秒 4 次（run 41870361258），因此依規則拆出 `read_document`；
- 拆分後（run 41872491978）：
  - `open_document`：120 秒 17,237 次（每秒 142 次）；
  - `read_document`：120 秒 13,802 次（每秒 114 次）；
- 兩者都沒有崩潰。

CI 結果（Linux runner，2026-11-05，#428，run 41884061447）：

- `text_box_font`：併入 `inspect_font` 前 120 秒 3,843 次（每秒 31 次，run 41874594705）；併入後 120 秒 3,991 次（每秒 32 次），覆蓋率由 9,342 edges 增至 9,608 edges，沒有崩潰；
- `stamp_image`（新目標）：120 秒 29,638 次（每秒約 244 次），覆蓋率 7,345 edges，沒有崩潰。

## 崩潰的處理

1. **下載並解密**：從失敗的 workflow 下載 `fuzz-crash-<目標>`，在 repo 以外的資料夾解開（PowerShell；先解密成檔案再解壓縮，因為 PowerShell 5.1 的管線會破壞二進位資料）：

   ```powershell
   age -d -i "$env:USERPROFILE\.age\pdf-reader-fuzz.txt" -o fuzz-crash.tar.gz fuzz-crash.tar.gz.age
   tar -xzf fuzz-crash.tar.gz
   ```

   崩潰樣本是 `fuzz/artifacts/<目標>/` 中的 `crash-*`、`oom-*`、`timeout-*` 檔案；`fuzz.log` 是完整輸出。
2. **重現**：

   ```bash
   cargo +nightly fuzz run <目標> <樣本檔>
   ```

3. **縮小**：

   ```bash
   cargo +nightly fuzz tmin <目標> <樣本檔>
   ```

4. **分類**：

   | 類型 | 判斷 | 處理 |
   |---|---|---|
   | Rust panic（`worker_messages`） | 主行程或 worker 的解碼／驗證 panic | 一般 bug，修正並加上單元測試 |
   | Rust panic（`open_document`） | 本專案程式碼（engine、scan、search）panic | 一般 bug；worker 會崩潰但被隔離，仍然要修 |
   | Rust panic（`create_pdf`） | 本專案程式碼（`PdfBuilder`、`jpeg`）panic | 一般 bug，修正並加上單元測試 |
   | AddressSanitizer 報告（MuPDF 內部） | 堆疊在 MuPDF 或第三方 C 函式庫 | **安全問題**：依 [SECURITY.md](../../SECURITY.md) 私下處理，確認是否已在上游修正，必要時升級 `mupdf` crate |
   | `oom-*`、`timeout-*` | 記憶體或時間超過上限 | 檢查是否有未設上限的配置或迴圈；worker 的 Job Object 記憶體上限與主行程逾時是最後防線 |

5. **加回迴歸測試**：修正後把縮小過的樣本變成測試。能公開的樣本放進 `tests/corpus/`（以產生器重現），不能公開的只留在私下的紀錄中。

**崩潰樣本可能就是可利用的攻擊檔案：不得附在 Issue、PR、討論或任何公開位置。**

## 主行程解析器測試（Property 與變異測試）

主行程沒有沙盒，自己也解析一些不可信或半可信的資料（攻擊面清單 `docs/security/attack-surface.md` §3，#429）。

### 為什麼不是 libFuzzer

- `src-tauri` 包含 Windows 專屬的 API（WebView2、WinHTTP、Win32 API 等），原始碼標有 `compile_error!`，只能在 Windows 環境編譯；
- CI 的 libFuzzer 工作（Linux runner）無法編譯或執行 `src-tauri`；
- 在 Windows 本機上，MSVC 連結器欠缺 libFuzzer 所需的 section 符號，且 Windows MSBuild 建置的依賴沒有插樁。

### 測試方式與位置

在各模組的單元測試中，以真實有效的樣本為起點，進行全面的變異測試：
1. **每一種截斷長度**：從 0 到樣本長度（較長者均勻採樣），確認不完整的輸入不會 panic；
2. **每個位置變異一個位元組**：嘗試極端值（`0x00`、`0xff`、`0x7f`）與語法字元（`{`、`"`、`0`、`:` 等），確認損毀的格式不會 panic；
3. **固定種子的偽隨機輸入**：測試內寫幾行 xorshift 產生數百組隨機長度與隨機內容的輸入，不新增 crate 依賴；
4. **大小上限**：超過模組大小限制的輸入在解析前即被拒絕。

各模組測試位置：
- `src-tauri/src/update_check.rs`：`interpret_handles_arbitrary_and_mutated_inputs_without_panic`（HTTPS 回覆）
- `src-tauri/src/recovery.rs`：`recovery_handles_arbitrary_and_mutated_inputs_without_panic`（復原日誌）
- `src-tauri/src/settings.rs`：`settings_handles_arbitrary_and_mutated_inputs_without_panic`（`settings.json`）
- `src-tauri/src/recent.rs`：`recent_handles_arbitrary_and_mutated_inputs_without_panic`（`recent.json`）
- `src-tauri/src/sensitive.rs`：`sensitive_handles_arbitrary_and_mutated_inputs_without_panic`（`sensitive.json`）
- `src-tauri/src/fonts.rs`：`font_headers_and_metadata_handle_arbitrary_and_mutated_inputs_without_panic`（字型檔頭與 `metadata.json`）
- `src-tauri/src/local_data.rs`：`local_data_handles_arbitrary_and_mutated_inputs_without_panic`（大小上限檢查與讀寫）

### 怎麼跑

CI 的 `Rust (Windows)` 工作會在 Windows 上跑 `cargo test --workspace --locked`，每個測試都在數十毫秒至數秒內跑完。

本機執行指令：
```bash
cargo test -p pdf-reader --locked
```

## 驗證紀錄

- **偵測與保存**：以一個故意的 panic（輸入以 `QA03` 開頭時）確認 workflow 會失敗並保存樣本，驗證後移除。結果見 QA-03 的 PR。
