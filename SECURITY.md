# 安全政策

## 支援版本

只修正最新發行的版本（[Releases](https://github.com/winner0988/Pdf-reader/releases/latest)）：修正會出現在下一個版本，舊版本不另外修正，請更新到最新版本。app 不會自動更新；「⋯」→「設定…」的「檢查更新」可以確認有沒有新版本。

## 回報弱點

- **不要**在 Issue、PR、討論或任何公開位置附上可利用的惡意 PDF、PoC 或崩潰樣本。
- 請使用 GitHub 的 **Private vulnerability reporting**：到本 repo 的 [Security](https://github.com/winner0988/Pdf-reader/security) 分頁，選 **Report a vulnerability**。回報內容只有你與維護者看得到，樣本檔可以直接附在私下的回報中。
- 請說明影響範圍、重現步驟與使用的版本（「⋯」→「關於」中的版本；從原始碼建置的請附 commit）。
- 無法使用上述管道時，請聯絡專案負責人（GitHub：[@winner0988](https://github.com/winner0988)），先不要附上樣本，我們會提供私下傳送的方式。

## 範圍內

- 開啟 PDF 就能觸發的任何行為：程式碼執行、連網、讀寫檔案、啟動外部程式
- 封鎖機制被繞過：JavaScript、`/OpenAction`、`/AA`、`/Launch`、遠端資源、外部連結確認
- `pdf_worker` 隔離被突破，或 worker 崩潰拖垮主行程
- 前端取得檔案路徑、或存取超出授權範圍的檔案
- 任何未經使用者同意的網路請求或資料外流

## 設計上的已知邊界

- 使用者確認開啟的外部連結，由系統瀏覽器連網，不屬於本應用程式的連網行為。
- 作業系統帳號被入侵時，系統憑證庫中的密碼可能外洩（ADR 0017，取代 ADR 0006）。
- **WebView2 對遭 `frame-src` 封鎖的 `<iframe>` 之預先連線（#664）**：WebView2 在 CSP 的 `frame-src 'none'` 擋下指向外部的 `<iframe>` 之前，會在瀏覽器層先對該目標發起預先連線（`is_preconnect: true`），於 net log 留下目標網址、主機與來源三筆記錄。net log 只有這三筆預先連線、沒有 `URL_REQUEST`，但預先連線有沒有實際送出 DNS 查詢或建立連線還沒有確認（CI 上的 `.example` 網域本來就解析不到），其他可能觸發預先連線的元素（`<link rel="preconnect">`、`<a>` 的滑鼠懸停）也沒有測過。本應用程式不建立指向外部網址的 `<iframe>`，且 PDF 內容由隔離在 `pdf_worker` 的 MuPDF 繪製成圖片，不會成為 WebView 中的 DOM，目前沒有已知路徑能讓 PDF 觸發此行為。已在 `E2E (Windows)` 實機環境逐一測試六項 Chromium 網路預測／預先連線參數（`--disable-preconnect`、`--dns-prefetch-disable`、`--disable-features=NavigationRequestPreconnect`、`--enable-features=PreloadingHoldback`、`--disable-features=NetworkPrediction`、`--disable-features=SpeculativePreconnect`，其中 `--disable-features` 項與既有停用清單合併），皆未能消除這三筆預先連線。因此維持原有啟動參數與 CSP，並於 [`tests/e2e/csp.spec.ts`](tests/e2e/csp.spec.ts)（`FRAME_PRECONNECT`）僅對此三筆預先連線保留明確例外；其他 10 種前端對外請求與 [`tests/e2e/network.spec.ts`](tests/e2e/network.spec.ts) 仍維持零外部位址。
