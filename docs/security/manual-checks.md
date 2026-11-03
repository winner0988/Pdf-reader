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

## 3. 1.0 發行之前（B4-11）

**目的**：1.0 發行前，在真正的 Windows 上用 1.0 的安裝檔確認 [attack-surface.md](attack-surface.md) 裡自動化測試證明不了的部分。表中的「§」指 attack-surface.md 的章節。

### 準備

1. 在測試用的電腦（或另一個 Windows 使用者帳號）上，以 1.0 的安裝檔安裝：`pnpm bundle` 產出的 `target/release/bundle/nsis/*.exe`，或發行頁上的檔案。以下假設裝在預設的 `C:\Program Files\PDF Reader\`。
2. 第 1 節的 Process Monitor，另外從 Microsoft 官方網站下載 Sysinternals **Process Explorer**（`https://learn.microsoft.com/sysinternals/downloads/process-explorer`）。兩者都只在測試用電腦上執行。
3. 只用 repo 的 `tests/corpus/` 裡的檔案，不要用私人的 PDF 或真的憑證。app 資料夾是 `%LOCALAPPDATA%\io.github.winner0988.pdfreader\`。

### 檢查

| # | 檢查 | 做法 | 預期 | 對照 |
|---|---|---|---|---|
| 3.1 | 安裝版再做一次第 1、2 節 | 用安裝好的 app 照第 1、2 節做 | 同第 1、2 節 | §5、§7 |
| 3.2 | worker 的沙盒 | 開一份 PDF。以系統管理員身分執行 Process Explorer，開 `pdf_worker.exe` 的 Properties：**Security** 分頁看 Integrity 與 AppContainer，**Job** 分頁看限制。再執行 `icacls "C:\Program Files\PDF Reader\pdf_worker.exe"` 與 `icacls "C:\Program Files\PDF Reader\pdf-reader.exe"` | `pdf_worker.exe` 在 `pdf-reader.exe` 底下；Integrity 是 Low（Process Explorer 可能顯示成 AppContainer），有 AppContainer 的 SID；Job 只允許 1 個行程，記憶體上限 2 GiB；兩個檔案的 ACL 一樣，app 沒有為 worker 改 ACL | §2.1 |
| 3.3 | 只有按「檢查更新」才連網 | 第 1 節的 Process Monitor 設定，只看 Network 事件。啟動 app，開一份文件，放著 5 分鐘；再按「檢查更新」 | 按之前沒有任何事件；按之後只有 `pdf-reader.exe` 連到 `api.github.com` 的 443 port；`pdf_worker.exe` 與 `msedgewebview2.exe` 都沒有 | §5 |
| 3.4 | 沒有除錯埠，也沒有開發人員工具 | PowerShell 執行 `Get-ChildItem Env:WEBVIEW2*`，確認沒有設定 WebView2 的環境變數。從開始功能表啟動 app，執行 `Get-NetTCPConnection -State Listen`，對照 Process Explorer 裡 `pdf-reader.exe` 底下各行程的 PID。在 app 裡按 `F12` 與 `Ctrl+Shift+I` | 沒有 WebView2 的環境變數；app 的行程沒有在聽任何 port；開發人員工具不會出現 | §4、§8 |
| 3.5 | 附件只存不開 | 開 `tests/corpus/benign/attachment-text.pdf`，把附件另存，再執行 `Get-Content -Path <存好的檔案> -Stream Zone.Identifier`。開 `tests/corpus/malicious/attachment-exe.pdf`，試著另存它的附件 | 有 `ZoneId=3`；app 沒有開啟附件（Process Monitor 沒有 `Process Create`）；`.exe` 的附件先警告危險，確認之後才存 | §1.2 |
| 3.6 | 機敏文件 | 把一份語料檔另存一份副本，標成機敏，開著它：按 `Win+Shift+S` 截圖；選取文字複製，按 `Win+V` 看剪貼簿歷程；關閉後看開始畫面的最近的檔案；文字索引開著時搜尋它的字 | 截圖裡 app 的視窗是黑的或空白；複製的文字不在剪貼簿歷程；文件不在最近的檔案，也搜尋不到 | §1.6 |
| 3.7 | 記住的密碼 | 開 `tests/corpus/benign/encrypted-aes256.pdf`（密碼見 `tests/corpus/manifest.json`），勾選記住密碼。開 Windows 的認證管理員（控制台 → 認證管理員 → Windows 認證），或執行 `cmdkey /list`。再到 app 的設定忘記全部的密碼 | 項目名稱是 `PdfReader/password/v1/` 加一串雜湊，看不出檔名、路徑或內容；忘記之後項目全部消失 | §1.1、§3 |
| 3.8 | 簽章不留下金鑰、不連網 | 記下 `%APPDATA%\Microsoft\Crypto\Keys` 與 `%APPDATA%\Microsoft\Crypto\RSA` 裡的檔案。Process Monitor 只看 Network 事件，用 `tests/corpus/certificates/signer.pfx`（密碼見 `tests/corpus/README.md`）簽一份語料檔 | 簽好的副本開成新分頁，簽章有效、簽署人不受信任；兩個資料夾沒有多出檔案；`certutil -user -store My` 沒有多出測試憑證；沒有任何網路事件 | §1.10、§3 |
| 3.9 | 匯入字型與語言資料 | 匯入字型：一個改成 `.ttf` 的文字檔、一個空的 `.ttf`、一個大於 64 MiB 的檔案，最後一個正常的 TrueType 字型。語言資料照樣試三個壞的，最後把安裝資料夾 `tessdata\` 的 `eng.traineddata` 複製一份改名為 `test.traineddata` 匯入 | 壞的都被拒絕，app 資料夾的 `fonts\`、`tessdata\` 沒有多出檔案；正常的只複製到這兩個資料夾，原檔不變 | §1.11、§1.12、§3 |
| 3.10 | 批次 | 對 3 個語料檔做「去除中繼資料」，執行中關閉視窗；完成後看選的資料夾；登出再登入 | 執行中通知區有圖示，關掉視窗後批次繼續；只有選的資料夾多了副本與報告，原檔不變；登入後 app 沒有自動啟動（工作管理員的「啟動應用程式」沒有它） | §1.9、§6 |
| 3.11 | 正常關閉後沒有留下復原日誌 | 編輯一份文件、存檔、關閉 app；再開、編輯、不存檔，關閉時選不要儲存。每次關閉後看 app 資料夾的 `recovery\` | 兩次都沒有留下日誌 | §1.7、§6 |
| 3.12 | 連結與 Windows 設定 | 在設定裡開啟「預設應用程式」；按「檢查更新」後開啟發行頁 | 前者開啟 Windows 設定的預設應用程式頁，app 列在裡面；後者先顯示確認，再以預設瀏覽器開啟發行頁 | §1.3、§7 |
| 3.13 | 檔案關聯與解除安裝 | `reg query "HKLM\Software\Classes\PdfReader.Document\shell\open\command"`；解除安裝後看 `%LOCALAPPDATA%\Packages\pdfreader.worker`、app 資料夾，並再查一次登錄 | 命令是 `"C:\Program Files\PDF Reader\pdf-reader.exe" "%1"`，兩邊都有引號；解除安裝後 AppContainer 的資料夾與 HKLM 的登錄都沒有了；app 資料夾刪不刪和解除安裝畫面的選項一致，把結果寫進紀錄 | §6、§8 |

## 紀錄

| 日期 | 版本（commit） | 執行者 | 結果 | 備註 |
|---|---|---|---|---|
| | | | 尚未執行 | MVP-11 合併前需由負責人執行一次 |
| | | | 尚未執行 | 第 3 節（含 3.1 的第 1、2 節）：1.0 發行前由負責人以 1.0 的安裝檔執行一次 |
