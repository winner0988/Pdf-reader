# 人工安全檢查

自動化測試無法完整證明「沒有發生」的事：沒有網路請求、沒有啟動程式、沒有存取其他檔案。本文件列出需要由人在真正的 Windows 上執行一次的檢查。

每次執行後，在下方「紀錄」補一列。檢查失敗時開 Issue，加上 `security` 標籤。

## 1. 惡意語料不造成任何副作用（MVP-11）

**目的**：`tests/corpus/malicious/` 的每個檔案開啟後：

- 沒有執行腳本；
- 沒有網路請求（包括 DNS 查詢與 SMB 連線）；
- 沒有啟動外部程式；
- 沒有存取語料以外的檔案。

### 準備

1. 以 `pnpm bundle` 建置並安裝，或使用 `pnpm tauri build --no-bundle` 產生的 `target/release/pdf-reader.exe`，並確認旁邊有 `pdf_worker.exe`。
2. 從 Microsoft 官方網站下載 Sysinternals **Process Monitor**（`https://learn.microsoft.com/sysinternals/downloads/procmon`）。它需要系統管理員權限，只在測試用電腦上執行。
3. 關閉其他會連網或大量存取檔案的程式，以減少雜訊。

### 步驟

1. 以系統管理員身分啟動 Process Monitor，**Filter**（`Ctrl+L`）加入：
   - `Process Name` `is` `pdf-reader.exe` → Include
   - `Process Name` `is` `pdf_worker.exe` → Include
   - `Process Name` `is` `msedgewebview2.exe` → Include
   - `Process Name` `is` `System` → Include（SMB 連線由系統核心發出）
2. 工具列只保留 **Show Registry**、**Show File System**、**Show Network Activity**、**Show Process and Thread Activity**。
3. `Ctrl+X` 清除事件，`Ctrl+E` 開始擷取。
4. 啟動 app，依序開啟 `tests/corpus/malicious/` 的每個檔案（檔案 → 開啟，或拖放）。每個檔案：
   - 捲動到最後一頁；
   - 點一下頁面上的每個連結或按鈕位置（MVP-12 之前連結還不能點，照樣點一下）；
   - 打開「已封鎖的內容」明細，確認列出的類別與 `tests/corpus/manifest.json` 一致。
5. 全部開完後關閉 app，`Ctrl+E` 停止擷取。

### 判讀

| 檢查 | 做法 | 預期 |
|---|---|---|
| 沒有網路 | 只顯示 Network 事件（`Operation` `begins with` `TCP`／`UDP`） | `pdf_worker.exe` 與 `pdf-reader.exe` 沒有任何事件；`msedgewebview2.exe` 沒有連到 `example.invalid` 或任何外部位址 |
| 沒有 SMB／UNC | `Path` `begins with` `\\`，以及 `System` 行程連到 445 port 的事件 | 沒有 `\\share.example.invalid` 或其他 UNC 路徑 |
| 沒有啟動程式 | `Operation` `is` `Process Create` | 只有 app 啟動 `pdf_worker.exe` 與 WebView2 自己的行程；沒有 `does-not-exist.example.exe` 或其他程式 |
| 沒有讀取其他檔案 | `Process Name` `is` `pdf_worker.exe`，`Operation` `is` `CreateFile` | 只有系統 DLL、字型與 worker 自己；**沒有**語料目錄或使用者目錄下的任何檔案（worker 只從繼承的檔案代號讀取文件，見 ADR 0008） |
| 沒有執行腳本 | 畫面 | 沒有任何對話框（樣本的腳本會呼叫 `app.alert`）；`openaction-uri.pdf` 沒有開啟瀏覽器 |

### 已自動化的部分

以下由 CI 或腳本持續檢查，但不能取代上面的人工檢查：

- worker 在 AppContainer 中執行，沒有任何網路 capability；Job Object 限制只能有 1 個行程，無法啟動子行程（`crates/sandbox/tests/sandbox.rs`、`crates/pdf_worker/tests/isolation.rs`）。
- worker 執行檔不匯入任何網路 DLL（`worker_binary_imports_no_networking`）。
- 每個樣本的掃描結果與 manifest 一致（`crates/pdf_worker/tests/active_content.rs`）。
- 開發時的抽查（MVP-11，2026-09-24，release 建置）：以內建工具逐一開啟 21 個惡意樣本，每個檔案等待 2.5 秒。結果：
  - worker 沒有子行程；app 的行程樹只有 app、worker 與 WebView2；
  - app 與 worker 沒有任何 TCP 連線；
  - SMB 連線數維持 0；
  - DNS 快取中沒有樣本主機（`*.example.invalid`）的查詢紀錄。

## 2. 外部連結只在確認後、以系統預設程式開啟（MVP-12）

**目的**：確認「開啟」真的交給預設瀏覽器，而且只交出檢查過的網址。

1. 開啟 `tests/corpus/benign/external-https-link.pdf`，點頁面上的連結。
2. 對話框出現時按「取消」→ 什麼都不會發生。
3. 再點一次，按「開啟」→ 預設瀏覽器開啟 `https://example.invalid/docs`（`.invalid` 是保留網域，瀏覽器會顯示無法連線）。
4. 依序點 `link-javascript-scheme.pdf`、`link-file-scheme.pdf`、`link-unc-uri.pdf`、`link-smb-scheme.pdf`、`link-ms-protocol.pdf`、`launch.pdf` 的連結 → 都只出現「已封鎖這個連結」，沒有任何程式啟動（可搭配第 1 節的 Process Monitor 設定，確認沒有 `Process Create`）。

開發時（MVP-12b，2026-09-24，release 建置）第 3 步已經以 CDP 點擊實際執行過一次：`ShellExecuteW` 回報成功，對話框隨之關閉。當時沒有確認瀏覽器畫面，仍需人工檢查一次。

## 紀錄

| 日期 | 版本（commit） | 執行者 | 結果 | 備註 |
|---|---|---|---|---|
| | | | 尚未執行 | MVP-11 合併前需由負責人執行一次 |
