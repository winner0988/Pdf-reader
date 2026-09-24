# 離線驗證流程

證明應用程式「不連網、零遙測」（AGENTS.md 原則 1、2；ADR 0009）。自動檢查在每個 PR 上執行；手動檢查在**每次發布前**執行，結果附在發布 PR 中。

## 自動檢查（CI）

| 層 | 檢查 | 位置 |
|---|---|---|
| 原始碼 | 遙測 SDK、遠端字型／CDN、具網路能力的 Tauri 外掛名稱 | `scripts/ci/check-forbidden.sh`（`Guardrails`） |
| Rust 依賴圖 | 禁用 HTTP／WebSocket／TLS client 與遙測 crate；授權白名單；只允許 crates.io | `deny.toml`（`Security` → `Dependency audit`） |
| npm 依賴 | 已知弱點 | `pnpm audit --prod`（`Dependency audit`） |
| 正式版 CSP | 只允許 `'self'`、`data:`／`blob:`（限圖片與 worker）、Tauri IPC；必須有 `object-src`／`base-uri`／`form-action`／`frame-src 'none'` | `scripts/ci/check-security-config.mjs`（`Guardrails`） |
| 開發模式 CSP | 同上，另外只允許 Vite 需要的 `'unsafe-inline'`（script／style）與 `ws://localhost:1420` | 同上（讀 `vite.config.ts`） |
| Capability | 前端權限必須在白名單內；`http:`、`shell:`、`fs:`、`opener:` 等永遠不可授予；不得設定 `remote` | 同上 |
| 前端 ESLint | 禁止 `fetch`、`XMLHttpRequest`、`WebSocket`、`EventSource`、`navigator.sendBeacon` | `eslint.config.js`（`Frontend`） |
| 建置產物 | `dist/` 的 HTML／CSS 不得引用外部資源 | `scripts/ci/check-dist.mjs`（`Frontend`） |

> 為什麼開發模式也要 CSP：Tauri 只把 `tauri.conf.json` 的 CSP 套用到自己提供的頁面，**不會**套用到 `devUrl`（Vite dev server）。MVP-01 實測發現開發模式下外部請求真的會送出，因此改由 `vite.config.ts` 的 dev server 自行送出 CSP，並納入上述檢查。

## 手動檢查（每次發布前）

### 準備

1. 以 `pnpm tauri build` 建置正式版，或安裝即將發布的安裝檔。
2. 準備 QA-01 的測試語料（`tests/corpus/`），**不要**使用私人文件。
3. 關閉其他使用網路的程式，降低干擾。

### 方法 A：連線監看腳本（內建工具，不需下載）

`scripts/security/watch-connections.ps1` 每 200 ms 列出 `pdf-reader.exe`、`pdf_worker.exe` 及其子行程（WebView2 的 `msedgewebview2.exe`）擁有的 TCP 連線與 UDP 端點。

```powershell
powershell -ExecutionPolicy Bypass -File scripts/security/watch-connections.ps1 -Seconds 180
```

在腳本執行期間依序操作下方「操作清單」。**預期結果：`Endpoints seen: 0`。**

限制：輪詢會漏掉比間隔更短的連線；DNS 查詢由系統的 DNS Client 服務發出，不屬於應用程式行程，此方法看不到。因此發布前還要做方法 B。

### 方法 B：Process Monitor（完整紀錄）

1. 從 Microsoft Sysinternals 官方網站取得 Process Monitor。
2. Filter：`Process Name` 是 `pdf-reader.exe`、`pdf_worker.exe`、`msedgewebview2.exe` → Include；`Operation` begins with `TCP` 或 `UDP` → Include。
3. 開始擷取後啟動應用程式，執行下方「操作清單」。
4. **預期結果：沒有任何事件。** 若 `msedgewebview2.exe` 出現事件，確認其父行程是否為本應用程式（其他程式也會使用 WebView2）。

### 操作清單

1. 啟動應用程式，停留 30 秒。
2. 開啟一般 PDF，捲動到最後一頁再回到第一頁。
3. 縮放、旋轉、開啟目錄側欄。
4. 搜尋一個存在與一個不存在的字詞。
5. 點擊文件中的外部連結，在確認對話框中按**取消**。
6. 依序開啟 QA-01 `malicious/` 目錄中的每個檔案。
7. 關閉應用程式。

> 功能尚未實作的步驟（MVP 期間）標記為「不適用」即可，但步驟 1 與 7 每次都要做。

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
