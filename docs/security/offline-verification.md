# 離線驗證流程

證明應用程式「不會自行連網、零遙測」（AGENTS.md 原則 1、2；ADR 0009）：唯一的連網是使用者在設定中按下「檢查更新」時的一個請求（[update-check.md](../architecture/update-check.md)）。自動檢查在每個 PR 上執行；手動檢查在**每次發布前**執行，結果附在發布 PR 中。

## 自動檢查（CI）

| 層 | 檢查 | 位置 |
|---|---|---|
| 原始碼 | 遙測 SDK、遠端字型／CDN、具網路能力的 Tauri 外掛名稱 | `scripts/ci/check-forbidden.sh`（`Guardrails`） |
| 原始碼 | WinHTTP（檢查更新用的唯一網路 API）只能出現在 `src-tauri/src/update_check.rs`（`@only` 規則） | 同上 |
| Rust 依賴圖 | 禁用 HTTP／WebSocket／TLS client 與遙測 crate；授權白名單；只允許 crates.io | `deny.toml`（`Security` → `Dependency audit`） |
| npm 依賴 | 已知弱點 | `pnpm audit --prod`（`Dependency audit`） |
| 正式版 CSP | 只允許 `'self'`、`data:`／`blob:`（限圖片與 worker）、Tauri IPC；必須有 `object-src`／`base-uri`／`form-action`／`frame-src 'none'` | `scripts/ci/check-security-config.mjs`（`Guardrails`） |
| 開發模式 CSP | 同上，另外只允許 Vite 需要的 `'unsafe-inline'`（script／style）與 `ws://localhost:1420` | 同上（讀 `vite.config.ts`） |
| WebView2 參數 | 每個視窗的 `additionalBrowserArgs` 都要有 `--disable-background-networking`、`--no-proxy-server`、`--disable-component-update`，並保留 Tauri 預設停用的功能；不得有遠端偵錯埠或 proxy（見下方「WebView2 的背景連線」） | `scripts/ci/check-security-config.mjs`（`Guardrails`） |
| 執行中的 app | WebView2 瀏覽器行程的命令列帶有 app 的每個參數；啟動、顯示文件並停留到啟動後 25 秒（元件更新程式以 `--component-updater=fast-update` 提早到約 10 秒詢問），WebView2 的 net log 沒有任何對外請求 | `tests/e2e/network.spec.ts`（`E2E (Windows)`） |
| Capability | 前端權限必須在白名單內；`http:`、`shell:`、`fs:`、`opener:` 等永遠不可授予；不得設定 `remote` | 同上 |
| 前端 ESLint | 禁止 `fetch`、`XMLHttpRequest`、`WebSocket`、`EventSource`、`navigator.sendBeacon` | `eslint.config.js`（`Frontend`） |
| 建置產物 | `dist/` 的 HTML／CSS 不得引用外部資源 | `scripts/ci/check-dist.mjs`（`Frontend`） |

> 為什麼開發模式也要 CSP：Tauri 只把 `tauri.conf.json` 的 CSP 套用到自己提供的頁面，**不會**套用到 `devUrl`（Vite dev server）。MVP-01 實測發現開發模式下外部請求真的會送出，因此改由 `vite.config.ts` 的 dev server 自行送出 CSP，並納入上述檢查。

## WebView2 的背景連線（#121）

WebView2 預設會**自行連網**，與 app 的程式碼無關：

- 啟動時向微軟的設定與實驗服務（`config.edge.skype.com`）取得設定，請求帶有用戶端識別碼、作業系統版本、安裝日期等參數；
- 以 WPAD 自動偵測 proxy（查詢 `wpad`）；
- 啟動後約 60 秒，元件更新程式（component updater）向 `edge.microsoft.com/componentupdater` 詢問 WebView2 元件的更新。

`src-tauri/tauri.conf.json` 的 `additionalBrowserArgs` 擋掉這三者（2026-09-30 以 WebView2 154 的 net log 逐一確認）：

| 參數 | 作用 |
|---|---|
| `--disable-background-networking` | 停用背景的網路請求，包括上面的設定服務。**不包括**元件更新程式 |
| `--no-proxy-server` | 不使用 proxy，也就不做 WPAD。WebView 只載入 app 內建的內容（`tauri.localhost`），不需要 proxy；檢查更新由主行程的 WinHTTP 發出，不受影響 |
| `--disable-component-update` | 停用元件更新程式：不詢問、也不下載元件（例如追蹤防護清單、憑證撤銷清單）。WebView 只顯示 app 內建的內容，用不到這些元件；WebView2 Runtime 本身的版本更新由 Microsoft Edge Update 負責，不受影響 |
| `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection` | Tauri 的預設（自訂參數會取代預設，所以要保留）：停用迷你選單、PDF 的迷你選單與 SmartScreen（SmartScreen 會把網址送到微軟檢查） |

- #124 只加了前兩個參數：當時的觀察只有 33 秒，沒等到約 60 秒時的元件更新程式。
- WebView2 更新版本後可能加入新的背景連線，所以除了 CI 的 `network.spec.ts`，發布前的手動檢查也要做。
- 設定服務不一定每次啟動都會連：2026-09-30 同一台電腦上，早先每次啟動都看得到它，幾小時後即使用全新的 WebView2 資料夾也看不到。所以沒看到它，不代表參數有效。元件更新程式則在每一次觀察中都會詢問。

## 手動檢查（每次發布前）

### 準備

1. 安裝即將發布的安裝檔：`pnpm bundle` 的產物，或草稿 Release 的附件（[release.md](../release.md)）。`pnpm tauri build` 的產物不含 worker，不要用。
2. 準備 QA-01 的測試語料（`tests/corpus/`），**不要**使用私人文件。
3. 關閉其他使用網路的程式，降低干擾。

### 方法 A：連線監看腳本（內建工具，不需下載）

`scripts/security/watch-connections.ps1` 每 200 ms 列出 `pdf-reader.exe`、`pdf_worker.exe` 及其子行程（WebView2 的 `msedgewebview2.exe`）擁有的 TCP 連線與 UDP 端點。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/security/watch-connections.ps1 -Seconds 180
```

在腳本執行期間依序操作下方「操作清單」。**預期結果**：

- 除了第 8 步（檢查更新），沒有任何端點；
- 第 8 步只出現一個 `pdf-reader` 的 TCP 連線，連到 GitHub（`api.github.com` 的位址）的 443 埠；有設定 WinHTTP proxy（`netsh winhttp`）時連到 proxy；
- 所以結果是 `Endpoints seen: 1`（沒做第 8 步則是 0）。

限制：輪詢會漏掉比間隔更短的連線（腳本以 `netstat` 輪詢，每輪約 0.1 秒；#64 之前每輪約 1 秒，漏掉了 #121 的連線）；DNS 查詢由系統的 DNS Client 服務發出，不屬於應用程式行程，此方法看不到。因此發布前還要做方法 B。

### 方法 B：Process Monitor（完整紀錄）

1. 從 Microsoft Sysinternals 官方網站取得 Process Monitor。
2. Filter：`Process Name` 是 `pdf-reader.exe`、`pdf_worker.exe`、`msedgewebview2.exe` → Include；`Operation` begins with `TCP` 或 `UDP` → Include。
3. 開始擷取後啟動應用程式，執行下方「操作清單」。
4. **預期結果**：只有第 8 步 `pdf-reader.exe` 連到 GitHub（或 WinHTTP proxy）443 埠的 TCP 事件，沒有其他事件。若 `msedgewebview2.exe` 出現事件，確認其父行程是否為本應用程式（其他程式也會使用 WebView2）。

### 操作清單

1. 啟動應用程式，停留 90 秒（WebView2 的元件更新程式預設在啟動後約 60 秒詢問，見上方「WebView2 的背景連線」）。
2. 開啟一般 PDF，捲動到最後一頁再回到第一頁。
3. 縮放、旋轉、開啟目錄側欄。
4. 搜尋一個存在與一個不存在的字詞。
5. 點擊文件中的外部連結，在確認對話框中按**取消**。
6. 依序開啟 QA-01 `malicious/` 目錄中的每個檔案。
7. 開啟「⋯」→「設定…」，**不要**按「檢查更新」，停留 30 秒。
8. 按一次「檢查更新」，等結果出現後再停留 30 秒（不應再有連線）。
9. 關閉應用程式。

> 功能尚未實作的步驟（MVP 期間）標記為「不適用」即可，但步驟 1、8 與 9 每次都要做。

### 紀錄格式

在發布 PR 中附上：

| 項目 | 內容 |
|---|---|
| 版本／commit | |
| 作業系統與 WebView2 版本 | |
| 方法 A 結果 | `Endpoints seen: N` |
| 方法 B 結果 | 事件數與說明 |
| 執行者與日期 | |

## 日誌政策

- 不得有任何崩潰回報、分析或日誌上傳；也不得自行實作「回報問題」功能自動送出資料。
- 日誌只寫在本機，由使用者自行決定是否提供。
- 正式建置的日誌不得包含檔案路徑、檔名以外的路徑片段、文件內容、搜尋字串或密碼。
- 目前（MVP-13）應用程式沒有任何日誌功能；新增日誌時須遵守本節，並加上 `needs-security-review`。

## 最近一次驗證

| 日期 | 版本 | 方法 | 結果 |
|---|---|---|---|
| 2026-09-24 | `main`（MVP-01 骨架，正式版建置） | 方法 A，啟動後停留 20 秒 | 0 個端點（WebView2 子行程已納入監看）；另以本機 TCP 連線做正向對照，腳本可正確偵測 |
| 2026-09-30 | #64 的分支（release 建置，E2E 以 CDP 操作） | 方法 A（改用 `netstat` 後的腳本）：開啟設定停留 20 秒不按，按一次「檢查更新」，再停留 20 秒 | 按下後只有一個 `pdf-reader` 的連線，連到 `api.github.com`（`20.27.177.116:443`），結果「GitHub 上還沒有任何發行版本」；前後都沒有其他 `pdf-reader` 的連線。另外兩條 `127.0.0.1` 是 E2E 的 CDP 連線。**但 WebView2 啟動時自行連到微軟（`config.edge.skype.com`），並做 WPAD 查詢**，一般啟動也會：見 #121，這不是本次變更造成的，修正前本流程不會通過 |
| 2026-09-30 | #121 的分支（release 建置，一般啟動：沒有 WebView2 環境變數，暫存的資料資料夾） | 方法 A，啟動後停留 33 秒；另查 WebView2 瀏覽器行程的命令列 | 0 個端點；命令列有 `--disable-background-networking` 與 `--no-proxy-server`。修正前以 net log 看到的設定服務請求與 WPAD 查詢都沒有了（`network.spec.ts` 也拿掉參數做過對照：會失敗） |
| 2026-09-30 | #121 的後續（release 建置；WebView2 參數以環境變數加上 `--log-net-log`，全新的 WebView2 資料夾） | WebView2 的 net log：只有 #124 的兩個參數時停留 90 秒；加上 `--disable-component-update` 後停留 6 分鐘，另以 `--component-updater=fast-update` 停留 40 秒 | 只有 #124 的參數時，約 60 秒連到 `edge.microsoft.com/componentupdater`（`fast-update` 時約 10 秒）；加上 `--disable-component-update` 後，兩種情況都沒有任何對外請求 |
| 2026-10-08 | `main`（fc8f9c1，release 建置；由 E2E 的測試程式碼操作，不是人工） | 方法 A，操作清單的第 1–7 與 9 步，共 240 秒：啟動後閒置 100 秒、捲動、搜尋一個不存在的字詞、依序開啟並關閉 `benign/` 與 `malicious/` 的每個檔案（41 個）、開啟設定停留 30 秒；**沒有做第 8 步**（不按「檢查更新」） | 2 個端點，都是 `msedgewebview2` → `127.0.0.1`（E2E 自己的除錯連線）；沒有任何非本機的端點。人工執行時不帶除錯連線，應是 0 個。這只是開發時的抽查，**不取代**發行前由人工在要發行的安裝檔上做的檢查（[release.md](../release.md)） |
