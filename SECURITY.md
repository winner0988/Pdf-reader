# 安全政策

## 支援版本

專案尚未發布任何版本，目前只修正 `main`。

## 回報弱點

- **不要**在 Issue、PR、討論或任何公開位置附上可利用的惡意 PDF、PoC 或崩潰樣本。
- 請使用 GitHub 的 **Private vulnerability reporting**：到本 repo 的 [Security](https://github.com/winner0988/Pdf-reader/security) 分頁，選 **Report a vulnerability**。回報內容只有你與維護者看得到，樣本檔可以直接附在私下的回報中。
- 請說明影響範圍、重現步驟與使用的版本（commit）。
- 無法使用上述管道時，請聯絡專案負責人（GitHub：[@winner0988](https://github.com/winner0988)），先不要附上樣本，我們會提供私下傳送的方式。

## 範圍內

- 開啟 PDF 就能觸發的任何行為：程式碼執行、連網、讀寫檔案、啟動外部程式
- 封鎖機制被繞過：JavaScript、`/OpenAction`、`/AA`、`/Launch`、遠端資源、外部連結確認
- `pdf_worker` 隔離被突破，或 worker 崩潰拖垮主行程
- 前端取得檔案路徑、或存取超出授權範圍的檔案
- 任何未經使用者同意的網路請求或資料外流

## 設計上的已知邊界

- 使用者確認開啟的外部連結，由系統瀏覽器連網，不屬於本應用程式的連網行為。
- 作業系統帳號被入侵時，系統憑證庫中的密碼可能外洩（ADR 0006）。
