# 公開 PDF 語料庫健壯性掃描 (Corpus Sweep)

對應工作卡 B5-02（Issue #439）；ADR 0008：隔離能限制損害，但不能取代找出問題。
模糊測試（見 [Fuzzing](./fuzzing.md)）使用變異輸入探索邊界與未知路徑，本掃描則聚焦於業界公開已知的大型真實與測試 PDF 語料庫，透過沙盒化工作行程（`pdf_worker`）進行批次健全性掃描。

> [!IMPORTANT]
> 語料庫中的 PDF 檔案屬於**不受信任的檔案**（untrusted inputs）。主行程（`corpus_sweep` runner）只負責走訪目錄並以唯讀模式開啟檔案控制代碼（File Handle），所有 PDF 解析、頁面渲染與文字萃取一律在低權限沙盒化的 `pdf_worker` 行程中執行，主行程絕對不解析 PDF 內容。

## 語料庫清單

掃描納入三個業界代表性公開 PDF 測試集，清單定義於 `tests/corpus-sweep/corpora.tsv`，鎖定特定版本／commit 並以 SHA-256 清單驗證：

| 語料名稱 | 來源專案 | Commit | 範圍 | 檔案數 | 總大小 | 授權 | 說明 |
|---|---|---|---|---|---|---|---|
| `pdf-differences` | [pdf-association/pdf-differences](https://github.com/pdf-association/pdf-differences) | `907fe96e52b73e491489eee545c47b119bf9989b` | `*.pdf` | 37 | 1,223,762 位元組 | Apache-2.0 | 涵蓋各類 PDF 邊緣案例、語法差異與畸形檔案 |
| `pdfjs` | [mozilla/pdf.js](https://github.com/mozilla/pdf.js) | `0a4153349dc473ba950fc17941c5ca7eda43c1de` | `test/pdfs/*.pdf` | 1,077 | 155,949,973 位元組 | Apache-2.0（個別檔案的授權沒有逐一確認，所以只下載來讀，不提交、不放進 artifact、不轉發） | Mozilla PDF.js 官方測試集，涵蓋廣泛真實世界特性與回歸案例 |
| `qpdf` | [qpdf/qpdf](https://github.com/qpdf/qpdf) | `4eba95899886e851cc41d76886483b347612f2a8` | `*.pdf` | 717 | 38,725,819 位元組 | Apache-2.0 | QPDF 測試套件，涵蓋各種物件壓縮、xref 損毀、加密與線性化結構 |
| **總計** | | | | **1,831** | **195,899,554 位元組** | | |

> [!NOTE]
> 為什麼 `pdfjs` 與 `qpdf` 檔案不直接提交至 Git repository：
> 1. 檔案總容量約 195 MB，若納入版本控制會造成 repository 膨脹。
> 2. 避免將包含各種惡意邊界測試或已修補 CVE 樣本的檔案常態存放於原始碼樹中觸發資安掃描誤報。
> 3. 因此以 `tests/corpus-sweep/corpora.tsv` 記錄來源 repo 與 commit，並以 `tests/corpus-sweep/<name>.sha256` 記錄個別檔案校驗碼，由 `scripts/corpus-sweep/fetch.sh` 隨需下載並驗證每個檔案之 SHA-256。

## 掃描架構與結果分類

`crates/worker_host/src/sweep.rs` 定義了掃描核心邏輯與結果回報結構。每個檔案開始前均建立獨立的全新 `WorkerHost`（用完即釋放，避免狀態或記憶體累積）：
1. 主行程開啟檔案並取得唯讀檔案控制代碼（File Handle）。
2. 傳遞給 `WorkerHost`（沙盒化 `pdf_worker`）呼叫 `open()`。
3. 成功開啟後，依序渲染每一頁（`fit_scale(page, 1.0)`）以驗證繪圖與字型管線。
4. 依序讀取每一頁的文字內容（`page_text(page)`）以驗證文字解碼管線。
5. 單一檔案整體上限為 60 秒；單一頁面渲染或操作逾時上限為 30 秒（由 worker host 監控）。

### 結果類型定義

| 結果類型 | 定義與判定條件 |
|---|---|
| `ok` | 檔案完整開啟，所有頁面均正常完成 1.0 倍渲染與文字萃取。 |
| `open_failed` | 沙盒 worker 正常攔截並拒絕開啟，回傳已知的錯誤代碼（如 `Corrupted`、`Encrypted`、`PasswordRequired`、`NotPdf` 等）。 |
| `render_failed` | 文件開啟成功，但其中某一頁在渲染階段回傳錯誤（Worker 仍正常存活）。 |
| `get_text_failed` | 文件開啟成功，但其中某一頁在文字萃取階段回傳錯誤（Worker 仍正常存活）。 |
| `worker_crashed` | Worker 行程在處理該檔案期間異常終止（崩潰／panic／記憶體錯誤）。`WorkerHost` 偵測到連線中斷。 |
| `worker_timeout` | 單頁渲染或操作超過 30 秒，Worker 未能如期回應。`WorkerHost` 強制終止 Worker。 |
| `protocol_violation` | Worker 回應無法解析的無效 IPC 訊息框或協定順序異常。 |
| `file_timeout` | 單一檔案的總處理時間超過 60 秒。 |
| `unreadable` | 主行程無法讀取該檔案（如權限不足或檔案不存在）。 |

## 在本機執行

### 1. 下載並驗證語料庫

使用 Git Bash 或 MSYS2 執行下載腳本（需指定目標目錄，無預設資料夾）：

```bash
# 下載並驗證所有語料庫（或在最後加上語料名稱只下載特定語料，如 pdf-differences）
bash scripts/corpus-sweep/fetch.sh corpus-files [pdf-differences]
```

腳本會：
- 下載原始碼封裝檔並解壓縮至暫存目錄，只保留範圍內的 PDF 檔案。
- 嚴格比對個別檔案之 SHA-256 與 `tests/corpus-sweep/<name>.sha256`。
- 檢查是否有非預期的額外檔案或遺漏檔案；驗證失敗立即中斷（exit code 1）。

### 2. 建置 Release 執行檔

掃描建議使用 release 建置以獲得完整效能（注意：僅建置 `pdf_worker` 與 `corpus_sweep`，不啟動完整 Tauri 應用程式）：

```bash
cargo build --release --locked -p pdf_worker --bin pdf_worker
cargo build --release --locked -p worker_host --bin corpus_sweep --features sweep
```

### 3. 執行掃描

```bash
target/release/corpus_sweep <語料資料夾> <輸出資料夾> [--worker <路徑>] [--file-seconds N] [--expect-files N] [--corpus <名字>] [--withhold-names]
```

例如：

```bash
target/release/corpus_sweep corpus-files sweep-output --worker target/release/pdf_worker.exe
```

輸出包含：
- `results.jsonl`：逐一記錄每個檔案的掃描結果、耗時、頁數與失敗階段。
- `summary.json`：結構化的統計數據（供程式或 CI 比對）。
- `summary.md`：GitHub 格式的統計報告與顯著問題（notable issues）清單。

## 自動化與 CI（`.github/workflows/corpus-sweep.yml`）

- **排程執行**：每週一 UTC 04:47（台灣時間 12:47）定期於 Windows runner 執行完整 1,831 個檔案的掃描。
- **手動觸發**：支援 `workflow_dispatch`，可手動啟動完整掃描。
- **PR 煙霧測試 (Smoke Test)**：針對修改 `crates/worker_host/Cargo.toml`、`crates/worker_host/src/sweep.rs`、`crates/worker_host/src/bin/corpus_sweep.rs`、`tests/corpus-sweep/`、`scripts/corpus-sweep/` 或該 workflow 本身的 PR，執行快速煙霧測試：
  - 掃描 repo 內建的 `tests/corpus/` 與體積較小的 `pdf-differences`（共 37 個檔案）。
  - 確保掃描器本身功能健全且能正確產生報告。
- **產出上傳與隱私防護**：
  - PR 與排程皆固定帶有 `--withhold-names` 旗標。CI 上 `worker_crashed`、`protocol_violation`、`worker_timeout`、`file_timeout` 僅顯示數量，不包含具體檔案名稱與細節。
  - 摘要報告直接印於工作記錄（Run Log）中。
  - 結果上傳為 artifact `corpus-sweep-results`（保留 30 天）。
  - 若在 CI 記錄中發現這幾類異常數量不是 0，需在本機環境重跑以確認檔案名稱，並依 `SECURITY.md` 私下處理。

## 異常與漏洞處理流程

掃描過程中若發現未預期的 Worker 崩潰（`worker_crashed`）、未捕獲的記憶體存取錯誤或潛在安全問題：

1. **嚴格遵守資安揭露政策**：依 [SECURITY.md](../../SECURITY.md) 私下向專案維護者回報。
2. **禁止公開攻擊樣本與細節**：
   - **不得**在 GitHub Issue、PR、討論區、commit 訊息或任何公開位置張貼觸發崩潰或逾時的原始檔案、檔名、完整呼叫堆疊（stack traces）或猜測原因。
   - 公開資訊僅記錄高階統計數量。
3. **隔離與留存**：
   - 將問題檔案隔離至本機私有專用目錄。
   - 由專案負責人依內部流程分析上游函式庫（MuPDF）修補情況並排定修正。

## 語料庫更新流程

當 upstream 專案發行新版本或需要納入新測試案例時：

1. 編輯 `tests/corpus-sweep/corpora.tsv`，更新目標 repo、commit hash、範圍或授權。
2. 執行 `fetch.sh --write-manifest <暫存目錄> [語料名稱]` 下載並自動產生新的個別檔案 SHA-256 清單（寫入 `tests/corpus-sweep/<name>.sha256`）。
3. 執行 `fetch.sh <暫存目錄> [語料名稱]` 確保校驗通過且無多餘檔案。
4. 執行完整本機掃描建立新的基準線，並在 PR 中更新本文檔之統計章節。

## 基準線掃描紀錄 (Baseline Run-1)

- **掃描日期**：2026-11-11（整次共用一個 worker 的舊版掃描程式）
- **執行環境**：Windows 11 (MSVC x64), Release 建置
- **檔案總數**：1,831 檔
- **防毒軟體狀態**：Windows Defender 實時防護開啟，全程無告警、無隔離或刪除任何檔案。

### 各語料庫結果統計

| 語料 (Corpus) | 總數 | 正常 (OK) | 開啟失敗 | 渲染失敗 | 萃取文字失敗 | 崩潰 (Crashed) | 逾時 (Timeout) | 協定錯誤 | 檔案逾時 | 無法讀取 |
|---|---|---|---|---|---|---|---|---|---|---|
| `pdf-differences` | 37 | 35 | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| `pdfjs` | 1077 | 1041 | 32 | 1 | 0 | 1 | 2 | 0 | 0 | 0 |
| `qpdf` | 717 | 654 | 61 | 2 | 0 | 0 | 0 | 0 | 0 | 0 |
| **合計 (Total)** | **1831** | **1730** | **95** | **3** | **0** | **1** | **2** | **0** | **0** | **0** |

### 開啟失敗錯誤碼統計

- **`pdf-differences`**：`Corrupted`: 2
- **`pdfjs`**：`Corrupted`: 19, `Encrypted`: 12, `NotPdf`: 1
- **`qpdf`**：`Corrupted`: 20, `Encrypted`: 25, `NotPdf`: 16

### 顯著案例紀錄 (Notable Findings)

1. 當掉 1 件、逾時 2 件，依 `SECURITY.md` 私下處理，這裡不列檔名。
2. **渲染失敗 (3 件)**：
   - `pdfjs`：`test/pdfs/issue19517.pdf`（第 1 頁，錯誤碼 `Internal`）。
   - `qpdf`：`qpdf/qtest/qpdf/issue-1688a.pdf`（第 1 頁，錯誤碼 `Internal`）。
   - `qpdf`：`qpdf/qtest/qpdf/issue-1688b.pdf`（第 1 頁，錯誤碼 `Internal`）。
   - 行為：Worker 正常存活並回傳錯誤，主行程如常記錄。
