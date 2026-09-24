# 端對端測試（E2E）

對應工作卡 QA-02。在 Windows 上啟動**真正的 app**（release 建置，含沙盒中的 `pdf_worker`），從外部操作它的畫面並檢查結果。

## 方案評估

| 方案 | 測什麼 | 需要什麼 | 評估 |
|---|---|---|---|
| **A. Tauri 官方 WebDriver**（`tauri-driver` + Microsoft Edge WebDriver） | 真正的 app | `cargo install tauri-driver`；`msedgedriver.exe` 的版本必須與執行環境的 WebView2 **完全相同**；WebDriver 用戶端（例如 WebdriverIO） | 官方支援，但每次 runner 更新 WebView2 都要下載對應版本的驅動程式，多兩個需要取得與固定版本的工具，CI 容易因版本不符失敗 |
| **B. Playwright 測前端＋模擬 IPC** | 只有 WebView 裡的 UI | Playwright 與瀏覽器 | 快，但測不到主行程、worker、沙盒與 IPC 驗證，也就是本專案最重要的部分；元件層面已經由 Vitest 涵蓋 |
| **C. Playwright 透過 CDP 連上真正的 app**（採用） | 真正的 app | 只有 `@playwright/test`（不下載任何瀏覽器或驅動程式）；WebView2 內建的遠端偵錯 | Microsoft 為 WebView2 app 記載的做法（[Playwright with WebView2](https://learn.microsoft.com/microsoft-edge/webview2/how-to/playwright)）。直接使用 runner 上已有的 WebView2 runtime，沒有版本對應問題 |

採用 **C**。B 不另外做：UI 的細節已經由 Vitest 元件測試負責，E2E 只放「整個 app 串起來」才能驗證的情境。

## 運作方式（`tests/e2e/app.ts`）

每個測試用 `launch(file?)` 啟動一個 app：

1. **環境變數**：WebView2 從環境變數讀取啟動設定。
   - `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<空閒的埠>`：遠端偵錯只綁 127.0.0.1。
   - `WEBVIEW2_USER_DATA_FOLDER=<新的暫存資料夾>`：每個測試有自己的 WebView2 瀏覽器行程與設定，不會連到前一個測試留下的行程，也不碰使用者的資料。
2. **開檔**：要開的檔案以命令列參數傳入（與使用者從檔案總管開啟相同的路徑，MVP-06）。只使用 `tests/corpus/` 的檔案。
3. **連線**：等偵錯埠回應後，以 `chromium.connectOverCDP` 連上，取得 app 視窗的頁面。
4. **結束**：中斷連線，以 `taskkill /T` 結束 app 與它的 worker、WebView2 行程，刪除暫存設定。
5. **失敗時**：保存截圖（`screenshot-N.png`）與 app 的日誌（stdout／stderr 與 WebView 主控台，`app-N.log`）到 `tests/e2e/test-results/`，也附在 HTML 報告中。

app 本身完全沒有為測試做任何修改：沒有測試專用的建置選項，也沒有開放遠端偵錯。遠端偵錯只在測試程式設定環境變數時才會開啟。

### 關於 WebView2 的環境變數

任何能設定 app 環境變數的程式，都能開啟它的 WebView2 遠端偵錯。這是 WebView2 的行為，不是本專案新增的能力。這種程式本身已經以使用者身分在電腦上執行，本來就能讀取使用者能讀的一切，所以這不在威脅模型內（見 ADR 0008 的信任邊界）。

## 目前的測試（`tests/e2e/open.spec.ts`）

| 情境 | 檢查 |
|---|---|
| 啟動 | 空狀態：標題、隱私說明、「開啟」按鈕 |
| 以命令列開啟 `benign/multi-page-10.pdf` | 狀態列顯示檔名與「第 1 / 10 頁」；第 1 頁已由 worker 渲染（`data-state="ready"`）；沒有安全警示橫幅 |
| 開啟 `malformed/page-tree-cycle.pdf` | 錯誤狀態：「這個 PDF 檔案已損毀，無法開啟。」與「開啟其他檔案」 |
| 開啟 `malformed/not-a-pdf.pdf` | 錯誤狀態：「這不是 PDF 檔案。」 |

之後每張功能卡都可以在 `tests/e2e/` 加上自己的驗收情境。預期文字一律從 `src/i18n/zh-TW.ts` 取得，不要寫死。

## 在本機執行

```bash
pnpm e2e:build
```

```bash
pnpm e2e
```

- `pnpm e2e:build` 建置 release 版的 `pdf_worker` 與 app（`target/release/`，不建置安裝檔）。
- 要測其他位置的 app：設定 `E2E_APP=<pdf-reader.exe 的路徑>`（旁邊要有 `pdf_worker.exe`）。
- 測試會開啟 app 視窗；執行期間不要操作滑鼠鍵盤。
- 報告：`tests/e2e/playwright-report/index.html`。

## CI

`CI` workflow 的 `E2E (Windows)` job：

- 建置 release app，執行 `pnpm e2e`。
- 失敗時上傳 `e2e-results` artifact（截圖、日誌、HTML 報告），**保留 7 天**。內容只會是 `tests/corpus/` 檔案的畫面。
- `retries: 0`：不穩定的 E2E 是要修的錯誤，不用重試掩蓋。
- 一次只跑一個 app（`workers: 1`）。

## 依賴

`@playwright/test` 1.63.0（npm，Microsoft 官方套件，devDependency，版本固定）。

- 只使用它的 CDP 連線與測試執行器，**不下載任何瀏覽器**（不執行 `playwright install`）。
- 安裝時沒有 postinstall 腳本。
