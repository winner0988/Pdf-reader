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

每個測試用 `launch(file?, options?)` 啟動一個 app：

1. **環境變數**：WebView2 從環境變數讀取啟動設定。
   - `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<空閒的埠>`：遠端偵錯只綁 127.0.0.1。
     - WebView2 把這個變數的參數**加在** app 自己的參數之後：app 照常傳入 `src-tauri/tauri.conf.json` 的 `additionalBrowserArgs`（例如讓 WebView2 不自行連網的參數，#121），所以測試的 app 與出貨的 app 相同。
       - 2026-09-30 以 WebView2 154 確認：只設偵錯埠時，瀏覽器行程的命令列仍有 app 的參數。#124 誤以為這個變數會取代 app 的參數，把它們又放進變數一次，已經拿掉。
     - `options.netLog` 另外加上 `--log-net-log`：WebView2 把它發出的每個請求寫進這個檔案（`network.spec.ts`）。
     - `options.browserArguments` 另外加上只用於這次啟動的參數，例如 `network.spec.ts` 的 `--component-updater=fast-update`。
     - `options.deviceScaleFactor` 另外加上 `--force-device-scale-factor`，像 Windows 的顯示比例（例如 150%）一樣縮放，與這台電腦的設定無關。
     - CDP 的模擬（`Emulation.setDeviceMetricsOverride`）做不到：它讓捲軸保持整數的 CSS 像素，重現不了 #83。
     - `options.dataDir` 沿用同一個測試中前一次啟動的資料資料夾（`dataDir(page)`），搭配 `quit(page)`：立即結束前一個 app（像當機一樣，不會在結束時存任何東西），再以同一個資料資料夾重新啟動，用來測試跨次啟動保留的資料（B2-12）。
   - `WEBVIEW2_USER_DATA_FOLDER=<新的暫存資料夾>`：每個測試有自己的 WebView2 瀏覽器行程與設定，不會連到前一個測試留下的行程，也不碰使用者的資料。
   - `PDF_READER_DATA_DIR=<另一個新的暫存資料夾>`：app 自己的資料（最近開啟的檔案，#73）也寫到這裡，不會寫進使用者的清單。這是 app 讀的環境變數（不是 WebView2 的），在 CI 以系統管理員執行時也有效。測試以 `dataDir(page)` 取得這個資料夾。
2. **開檔**：要開的檔案以命令列參數傳入（與使用者從檔案總管開啟相同的路徑，MVP-06）。只使用 `tests/corpus/` 的檔案。
3. **連線**：等偵錯埠回應後，以 `chromium.connectOverCDP` 連上，取得 app 視窗的頁面。
4. **等 app 的頁面出現**：等到 React 在 `#root` 中畫出東西，最多 60 秒，才把頁面交給測試。
   - 原因（#133）：在每個測試全新的 WebView2 設定中，CI 偶爾要 17～35 秒才載入 app 的頁面——app 的 HTML 那麼晚才到，之後的檔案與呼叫都只要幾毫秒。測試自己的逾時（15 秒）原本就在 app 還沒開始時用完，看起來像各種奇怪的失敗（頁面還是 `about:blank`、找不到空狀態的標題、頁面一直在「繪製中」）。
   - 等待的時間不算在測試的時間限制內：測試的限制加上這次啟動花的時間。
   - 頁面超過 5 秒才出現時，測試的輸出註明花了多久，以及當時的情況，用來確認延遲是在 WebView2／runner，還是 app 自己：
     - app 的視窗是否回應（`IsHungAppWindow`：app 的主執行緒是不是卡住）；
     - app 與它的 WebView2 各個行程用了多少處理器時間；
     - 這台電腦最忙的行程與磁碟。
5. **結束**：中斷連線，以 `taskkill /T` 結束 app 與它的 worker、WebView2 行程，刪除暫存設定與資料資料夾。
6. **失敗時**：保存截圖（`screenshot-N.png`）與 app 的日誌（stdout／stderr 與 WebView 主控台，`app-N.log`）到 `tests/e2e/test-results/`，也附在 HTML 報告中。
   - 測試的輸出（也就是 CI 的記錄）另外印出一段摘要，不必下載上傳的檔案就能先判斷（#133）：
     - WebView 在啟動後多久回應、app 的頁面多久出現、app 是否還在執行；頁面慢時的情況（見上一步）；
     - WebView 有哪些頁面、哪一個是測試操作的；
     - 頁面對主行程的呼叫（`invoke`）：各命令已回應幾次、哪些還在等待與等了多久，例如頁面有沒有要求繪製、主行程有沒有回應；
     - 頁面的網址、載入狀態、這份文件何時開始（與 app 啟動的時間比較，看得出頁面是否重新載入）、是否可見與有焦點、視窗大小、畫面上的文字（前 1,000 字）、各頁的繪製狀態，以及有沒有元素以 `aria-hidden`／`inert` 把狀態列或頁面藏起來；
     - 頁面的時間軸（頁面自己的 resource timing，從文件開始起算的毫秒）：app 自己的檔案何時到達、何時解析完與載入完成，以及每個對主行程的呼叫何時送出與回應；
     - 頁面的主執行緒：從測試取得頁面起，計時器或動畫影格停頓超過 0.25 秒的時刻與長度（測試在每份文件中放一個心跳，app 不讀它），以及各類工作花的時間（CDP `Performance`：腳本、版面、樣式、全部工作）；
     - 頁面自己的計時器、動畫影格與對主行程的呼叫（唯讀的 `get_settings`）是否還有回應，以及畫布寬度在十個影格中是否一直改變（符合寬度時，寬度一變就要重新繪製）；
     - 第一頁的頁面元件在 React 中的狀態（從 React 放在元素上的內部欄位讀取，只在失敗時）：是否在等縮放比例穩定（`paused`）、比例、延遲，以及半秒後是否仍是同一個元素（一直重新掛載的元件永遠不會要求繪製）；
     - 測試的定位器看到什麼：狀態列有幾個、頁面文字的前 200 字。
       - 定位器在 Playwright 自己的腳本世界中執行，與頁面的腳本分開。CI 上曾出現頁面文字已顯示文件、定位器卻連狀態列都找不到，頁面也一直沒有畫完的情況（#133），這幾項用來分辨原因；
     - app 日誌的最後 3,000 字。
   - WebView 根本沒有啟動時沒有頁面可截：
     - 在 CI 上改截 runner 的整個桌面，看得到 app 可能在等待的對話框；
     - 在開發者電腦上不截，因為那會截到你自己的螢幕。
   - 日誌另外附上 app 的行程樹與各行程的命令列。
   - WebView 沒有啟動時，錯誤訊息本身就附上 WebView2 瀏覽器行程的命令列：不必下載 CI 上傳的檔案，從記錄就看得出它收到了哪些參數。

app 本身完全沒有為測試做任何修改：沒有測試專用的建置選項，也沒有開放遠端偵錯。遠端偵錯只在測試程式設定環境變數（或 CI 上的機器原則，見下）時才會開啟。`PDF_READER_DATA_DIR` 是一般的設定（見 [recent-files.md](recent-files.md)），不是測試專用。

### CI 上的機器原則

- **原因**：WebView2 Runtime 150 起，**以系統管理員權限（elevated）執行的 app** 會忽略 `WEBVIEW2_*` 環境變數與目前使用者（HKCU）的原則所加的瀏覽器參數，只接受機器原則（HKLM）與 app 本身透過 API 傳入的參數。GitHub 的 Windows runner 以系統管理員執行，所以只設環境變數時，遠端偵錯埠不會開啟。
  - QA-02 初次在 CI 執行時就是這樣：app 正常顯示，但 WebView2 瀏覽器行程的命令列沒有 `--remote-debugging-port`。
  - runner 上是 WebView2 152；本機（一般權限，153）不受影響。
- **做法**：只在 CI（`CI` 環境變數）上，每次啟動 app 前，在 `HKLM\Software\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments` 寫入以執行檔名稱（`pdf-reader.exe`）為名的值，app 結束後刪除。
  - 值與環境變數的瀏覽器參數相同：`--remote-debugging-port=<埠>`，需要時再加上 `--force-device-scale-factor`、`--log-net-log` 等。它和環境變數一樣加在 app 自己的參數之後。
  - 值要短。#121 的第一版把 app 的參數也放進這個值，加上 `--log-net-log` 在暫存資料夾的長路徑，約 265 字元；CI 上的 WebView 就沒有帶著偵錯埠啟動（其他測試約 175 字元，都正常）。原因沒有確認，可能是長度上限。現在的值最長約 150 字元。
- **本機**：不會修改登錄檔，因為那是整台電腦的設定；只用環境變數。

### 關於 WebView2 的環境變數

任何能設定 app 環境變數的程式，都能開啟它的 WebView2 遠端偵錯。這是 WebView2 的行為，不是本專案新增的能力。這種程式本身已經以使用者身分在電腦上執行，本來就能讀取使用者能讀的一切，所以這不在威脅模型內（見 ADR 0008 的信任邊界）。

## 目前的測試（`tests/e2e/open.spec.ts`）

| 情境 | 檢查 |
|---|---|
| 啟動 | 空狀態：標題、隱私說明、「開啟」按鈕 |
| 以命令列開啟 `benign/multi-page-10.pdf` | 狀態列顯示檔名與「第 1 / 10 頁」；第 1 頁已由 worker 渲染（`data-state="ready"`）；沒有安全警示橫幅 |
| 開啟 `malformed/page-tree-cycle.pdf` | 錯誤狀態：「這個 PDF 檔案已損毀，無法開啟。」與「開啟其他檔案」 |
| 開啟 `malformed/not-a-pdf.pdf` | 錯誤狀態：「這不是 PDF 檔案。」 |
| 以開啟對話框開啟（#86） | `Ctrl+O` 後取消：沒有分頁；再按「選擇檔案…」，輸入 `benign/single-page.pdf` 並開啟：分頁出現、第 1 頁已渲染。對話框是系統的，由 `answerFileDialog`（`file-dialog.ps1`）以 UI Automation 找到後回答 |
| 匯出（B2-04，`export.spec.ts`） | 純文字寫到另存的檔案；兩頁 PNG 寫到選的資料夾（簽名、72 dpi 的寬度）；取消另存時什麼都不寫 |
| 不自行連網（#121，`network.spec.ts`） | WebView2 瀏覽器行程的命令列帶有 app 的每個參數；啟動、顯示文件並停留到啟動後 25 秒（以 `--component-updater=fast-update` 讓元件更新程式提早到約 10 秒詢問，預設約 60 秒；app 的 `--disable-component-update` 照樣停用它），WebView2 的 net log 除了 app 自己的內容，沒有任何請求 |
| 頁面管理（B2-05，`pages.spec.ts`） | 在縮圖上以右鍵刪除第 3 頁、以「移到…」把第 5 頁移到最前面、右鍵把第 2 頁向右轉；另存後重新開啟：9 頁，搜尋定位各頁、第 3 頁是橫的。另以滑鼠拖曳縮圖移動頁面 |
| 表單（B2-09，`forms.spec.ts`） | 填樣本表單的每一種欄位（文字、多行、最大長度、核取方塊、選項按鈕、兩種選單、必填），另存新檔後重新開啟，值都在；唯讀欄位只能看；有腳本的欄位可以填，狀態列說明腳本不執行，沒有對話框；扁平化後另存，頁面上沒有欄位，重新開啟的新檔搜尋得到填的值 |
| 數位簽章（B2-14，`signatures.spec.ts`） | 開啟 `benign/signed.pdf`：簽章提示列說簽章有效但無法確認簽署者，面板列出語料的測試憑證、簽署時間（標示為簽署者自己聲稱）與離線驗證的說明，關閉後焦點回到按鈕；`signed-docmdp-p1.pdf` 的面板說簽署者不允許任何變更；改動簽章範圍內一個字母的檔案顯示簽章無效、不列簽署者；沒有簽章的文件沒有提示列 |
| 儲存（B2-02，`saving.spec.ts`） | 還沒有編輯的 UI，所以以 app 自己的 IPC（`apply_edit`，文件 id 取自頁面的 `data-doc`）旋轉第 1 頁：另存新檔後重新開啟新檔，第 1 頁已旋轉、原檔的雜湊值不變；`Ctrl+S` 寫回原檔；`Ctrl+W` 詢問（取消、不儲存）；以 `closeAppWindow`（對 app 視窗送出 `WM_CLOSE`，同關閉按鈕）關閉視窗時詢問，選「儲存」後 app 結束、重新開啟時第 1 頁已旋轉 |

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
- 測試會開啟 app 視窗（有幾個也會開啟系統的開啟、另存或選擇資料夾對話框並自動回答）；執行期間不要操作滑鼠鍵盤。
- 報告：`tests/e2e/playwright-report/index.html`。

## CI

`CI` workflow 的 `E2E (Windows)` job：

- 建置 release app，執行 `pnpm e2e`。
- 失敗時上傳 `e2e-results` artifact（截圖、日誌、HTML 報告），**保留 7 天**。內容只會是 `tests/corpus/` 檔案的畫面。
- `retries: 0`：不穩定的 E2E 是要修的錯誤，不用重試掩蓋。
- 一次只跑一個 app（`workers: 1`），所以同一時間只有一個機器原則值。
- Rust 快取在測試失敗時也保存（`cache-on-failure`），修正測試時不必每次重新建置 MuPDF。

## 依賴

`@playwright/test` 1.63.0（npm，Microsoft 官方套件，devDependency，版本固定）。

- 只使用它的 CDP 連線與測試執行器，**不下載任何瀏覽器**（不執行 `playwright install`）。
- 安裝時沒有 postinstall 腳本。
