# Fuzzing

對應工作卡 QA-03；ADR 0008：隔離能限制損害，但不能取代找出問題。

## 目標

| 目標（`fuzz/fuzz_targets/`） | 測什麼 | 種子 |
|---|---|---|
| `worker_messages` | IPC 的兩端：主行程解碼並驗證 worker 的回應（`decode::<WorkerResponse>`、`check_hello`、`Validate`），worker 解碼主行程的請求，以及長度前綴的分框（`read_frame`） | `fuzz_seeds` 為每一種訊息寫一個有效的 payload |
| `open_document` | 以 MuPDF 開啟 PDF，再做 worker 會做的每件事：頁面尺寸、渲染（0.1 倍、旋轉 90°）、連結、搜尋（前兩頁）、目錄、主動內容掃描 | `tests/corpus/` 的 PDF（QA-01，不含隨需產生的大型檔） |

- 對所有輸入，程式都必須正常接受或拒絕：不能 panic、不能無限制地配置記憶體、不能出現記憶體錯誤。
- 任何一種都算崩潰，libFuzzer 會把觸發它的輸入存到 `fuzz/artifacts/<目標>/`。

## 自動執行（`.github/workflows/fuzz.yml`）

- **排程**：每週一，Linux runner，每個目標 20 分鐘。
- **PR**：修改 `fuzz/`、`crates/ipc_contract/`、`crates/pdf_worker/`、`tests/corpus/` 或這個 workflow 時，每個目標跑 2 分鐘。
- **手動**：Actions → Fuzz → Run workflow。
- **MuPDF 也被偵測**：
  - workflow 以 `CC=clang`、`CFLAGS=-fsanitize=fuzzer-no-link,address` 建置，MuPDF 與它的 C 函式庫也有覆蓋率回饋與 AddressSanitizer，不只 Rust 程式碼。
  - 只設 C 的旗標，因為 libFuzzer 本身是 C++，不能對自己插樁。
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
  - `.github/fuzz-recipient.txt` 還沒有 `age1…` 公鑰時，workflow 只發出警告，不上傳任何東西。
  - 這時要在本機重現（見「在本機執行」）。

#### 負責人的金鑰（只需做一次）

1. 安裝 age：`winget install FiloSottile.age`。
2. 在 repo 以外的位置產生金鑰（PowerShell）：

   ```powershell
   New-Item -ItemType Directory -Force "$env:USERPROFILE\.age" | Out-Null
   age-keygen -o "$env:USERPROFILE\.age\pdf-reader-fuzz.txt"
   ```

3. 把它印出的 `Public key: age1…` 中的 `age1…` 放進 `.github/fuzz-recipient.txt`，以 PR 提交。公鑰可以公開。**私鑰檔不得提交或傳給任何人**，請另外備份。

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

執行（例如 5 分鐘）：

```bash
cargo +nightly fuzz run open_document fuzz/corpus/open_document -- -max_total_time=300 -max_len=1048576 -rss_limit_mb=4096
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
- 兩者都沒有崩潰。

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
   | AddressSanitizer 報告（MuPDF 內部） | 堆疊在 MuPDF 或第三方 C 函式庫 | **安全問題**：依 [SECURITY.md](../../SECURITY.md) 私下處理，確認是否已在上游修正，必要時升級 `mupdf` crate |
   | `oom-*`、`timeout-*` | 記憶體或時間超過上限 | 檢查是否有未設上限的配置或迴圈；worker 的 Job Object 記憶體上限與主行程逾時是最後防線 |

5. **加回迴歸測試**：修正後把縮小過的樣本變成測試。能公開的樣本放進 `tests/corpus/`（以產生器重現），不能公開的只留在私下的紀錄中。

**崩潰樣本可能就是可利用的攻擊檔案：不得附在 Issue、PR、討論或任何公開位置。**

## 驗證紀錄

- **偵測與保存**：以一個故意的 panic（輸入以 `QA03` 開頭時）確認 workflow 會失敗並保存樣本，驗證後移除。結果見 QA-03 的 PR。
