# 主動內容與遠端引用的偵測

對應工作卡 MVP-11；ADR 0001（主動內容預設封鎖）、ADR 0002（信任模型）。

## 原則

- **什麼都不執行。** worker 的 MuPDF 編譯時就不含 JavaScript 引擎（`FZ_ENABLE_JS=0`，見 [mupdf-binding.md](mupdf-binding.md)；`javascript_is_compiled_out` 測試）。viewer 除了文件內的跳頁，不執行任何 PDF 動作。
- **掃描只是告知。** 開啟文件時，worker 掃描文件，回報擋下了哪些內容（`SecurityReport`）。前端顯示橫幅與明細；**沒有「允許執行」按鈕**，信任例外不在 MVP（ADR 0002）。
- **掃描也在解析不可信資料**，所以在 worker 內、沙盒中執行，並有時間與數量上限。

## 掃描方式（`crates/pdf_worker/src/scan.rs`）

- **走訪**：從文件目錄（catalog）出發，走訪所有可到達的字典與陣列。
  - 使用明確的堆疊，沒有遞迴，深層巢狀不會讓堆疊溢位。
  - 每個間接物件只走一次，循環在此結束。
  - 不讀取任何串流內容，只看串流的字典。
  - 每個字典的鍵值一次讀完再分類。逐鍵查詢會讓每個鍵都多一次 MuPDF 呼叫，是大型目錄主要的耗時來源。
- **上限**：最多 2,000,000 個物件、2 秒（`ScanBudget`）。
  - 超過時停止，回報已找到的內容，並設 `scanComplete = false`。
  - 前端顯示「文件太大，掃描未完成；可能還有未列出的項目。」
- 走不到的孤立物件不算：它們無論如何都不會被執行。

### 分類規則

| FindingKind | 條件 | 數量 |
|---|---|---|
| `javaScript` | `/S /JavaScript` 的動作；或其他帶 `/JS` 的字典（例如 Rendition 動作） | 每個動作 1 |
| `openAction` | 文件目錄的 `/OpenAction` 是動作字典，而且不是單純的 `/GoTo`（或 `/GoTo` 後面還有 `/Next`）。**只指定開啟頁面的目的地陣列不算**：一般 PDF 很常見，只是「從第幾頁開始」 | 1 |
| `additionalActions` | 任何字典的 `/AA`（文件、頁面、註解、表單欄位）裡的每個觸發項目 | 每個觸發 1 |
| `launch`、`submitForm`、`importData`、`remoteGoTo`、`embeddedGoTo` | `/S` 是 `Launch`、`SubmitForm`、`ImportData`、`GoToR`、`GoToE` | 每個動作 1 |
| `remoteFileSpec` | `/FS /URL` 的檔案規格（例如從網址載入資料的影像）。**屬於上述動作的 `/F` 不重複計算**：表單的送出網址就是 `submitForm` | 每個 1 |
| `uncReference` | 檔案名稱欄位（`/F`、`/UF`、`/DOS`、`/Unix`、`/Mac`）是網路路徑：`\\server\share`（含斜線寫法）、有主機的 `file://`、`smb:`。**與動作本身同時回報**，因為 Windows 存取 UNC 路徑可能送出帳號雜湊（SMB／NTLM） | 每個字典 1 |
| `xfa` | 帶 `/XFA` 的字典（AcroForm） | 1 |
| `richMedia` | `/Subtype` 是 `RichMedia`、`Screen`、`Movie`、`Sound`、`3D` 的註解；`/S /RichMediaExecute` | 每個 1 |
| `embeddedFile` | 帶 `/EF` 的檔案規格（內嵌附件、附件註解） | 每個 1 |

- `URI` 動作不算發現。點擊時由 MVP-12 的確認對話框處理，所以 `link-*.pdf` 樣本的預期是沒有發現。
- 同一個間接物件被多處引用時只算一次。
- 類別會重疊：頁面 `/AA` 裡的腳本同時算 `javaScript` 與 `additionalActions`。因此橫幅的數字是「類別數」，明細才列出各類的數量。

## 量測

開發者電腦（Windows 11），release 建置，只計掃描本身：

| 文件 | 掃描 | 結果 |
|---|---|---|
| `large-1000-pages.pdf`（197 MB，1000 頁，每頁一張影像） | 9 ms | 沒有發現 |
| `outline-100k-items.pdf`（10 萬個目錄項目） | 290 ms（改成一次讀完鍵值前是 445 ms） | 沒有發現 |
| 一頁含 210 萬個空陣列（手動產生，不提交） | 停在上限 | `scanComplete = false`，app 顯示掃描未完成 |

## 測試

- `crates/pdf_worker/tests/active_content.rs`：
  - 語料庫（`tests/corpus/manifest.json`）的每個樣本，發現的類別都與 manifest 一致；
  - 惡意樣本另外經由沙盒中的 worker 開啟，確認報告通過 IPC 驗證；
  - 各條規則的正反例：目的地型 `/OpenAction`、計數、共用物件與循環、UNC、送出網址、XFA／多媒體／附件、深層巢狀、數量與時間上限。
- `scan.rs` 單元測試：網路路徑判斷。
- 前端：`src/features/security-banner/`（摘要、排序、是否顯示）與 `ReaderShell.test.tsx`（橫幅、明細面板、掃描未完成）。
- 作業系統層級的人工檢查：[docs/security/manual-checks.md](../security/manual-checks.md)。
