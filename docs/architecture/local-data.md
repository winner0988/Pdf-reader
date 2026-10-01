# App 在這台電腦上保存的資料

工作卡 [#101](https://github.com/winner0988/Pdf-reader/issues/101)（B2-12）。設定頁的「這台電腦上保存的資料」就是這份清單（[screen-map.md](../ux/screen-map.md)「設定」）。

## 位置

全部在 `%LOCALAPPDATA%\io.github.winner0988.pdfreader\`（Tauri 的 `app_local_data_dir`）：

- 本機（Local）而不是漫遊（Roaming）的資料夾，不會隨網域的漫遊設定檔同步到其他電腦；
- 資料夾預設只有目前的 Windows 使用者（與系統管理員）能讀；
- app 不會上傳任何資料。

`PDF_READER_DATA_DIR` 設為絕對路徑時，改用那個資料夾（E2E 測試用來隔離每次執行，見 [e2e.md](e2e.md)）。

| 檔案或資料夾 | 內容 | 誰寫入 | 說明 |
|---|---|---|---|
| `recent.json` | 最近開啟的檔案的**完整路徑**（最多 20 筆）；「不記錄此檔案」的加鹽雜湊值 | 主行程 | [recent-files.md](recent-files.md) |
| `settings.json` | 外觀、是否記錄最近開啟的檔案 | 主行程 | 本頁「設定」 |
| `recovery\` | 崩潰復原日誌：有未儲存變更的文件，每份一個，含檔案的**完整路徑**、大小與修改時間，以及還沒存檔的編輯。正常存檔、不儲存或關閉時刪除，只有 app 沒有正常結束時才會留下 | 主行程 | [crash-recovery.md](crash-recovery.md) |
| `EBWebView\` | 畫面元件（WebView2）的暫存資料 | WebView2 | 本頁「WebView2」 |

另外，worker 的 AppContainer profile 在 `%LOCALAPPDATA%\Packages\` 底下，解除安裝時由安裝程式刪除（`src-tauri/windows/installer-hooks.nsh`）。

## 讀寫的規則（`src-tauri/src/local_data.rs`）

- **只有主行程**讀寫這些檔案。前端只能透過型別化的命令取得或修改已定義的項目，拿不到路徑，也不能寫入任意內容。
- **寫入**：先寫到旁邊的暫存檔，再改名取代，當機時不會留下半個檔案。
- **讀取**：
  - 有大小上限（`recent.json` 512 KiB、`settings.json` 64 KiB、每個復原日誌 4 MiB）；
  - 超過上限、不是 JSON、版本不對或有未知欄位時，當成沒有這個檔案，使用預設值；下次變更時覆寫。

## 設定（`src-tauri/src/settings.rs`）

| 項目 | 預設 | 說明 |
|---|---|---|
| `theme` | `system` | `system`（跟隨系統）、`light`、`dark` |
| `recordRecentFiles` | `true` | 關閉時清除目前的清單，之後開啟的檔案都不記錄；「⋯」選單也不再提供「不記錄此檔案」 |

- 啟動時就讀取，因為視窗一開始就需要外觀。
- 前端送回**完整的一組設定**（`Settings`，`deny_unknown_fields`）。
- 變更立即套用；檔案寫不進去時仍然套用，畫面說明「重新啟動後會回到之前的設定」。
- 所有分頁共用同一組設定（前端的 `SettingsProvider`）。

## WebView2

WebView2 在 `EBWebView\` 保存它自己的資料（快取等）。

- app 不使用 `localStorage`、`IndexedDB` 或 cookie 保存任何東西；
- 頁面只從 app 內建的資源載入（CSP，見 [ipc-contract.md](ipc-contract.md)），不會有網站的資料。

## 刪除

- 設定頁可以清除最近開啟的檔案，以及「不記錄此檔案」的選擇。清除最近開啟的檔案（或關閉記錄）時，一併刪除目前沒有分頁使用的復原日誌。
- 關閉 app 後，可以直接刪除整個資料夾：下次啟動時一切回到預設值。
