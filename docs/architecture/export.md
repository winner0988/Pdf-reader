# 匯出純文字與頁面圖片（B2-04、#111）

工作卡 [#93](https://github.com/winner0988/Pdf-reader/issues/93)（純文字、PNG）與 [#111](https://github.com/winner0988/Pdf-reader/issues/111)（JPG）；規格 §7「格式匯出」中的純文字與圖片（Word 另開 POC）。畫面見 [screen-map.md](../ux/screen-map.md)「匯出」。

## 流程

```mermaid
sequenceDiagram
  participant F as 前端
  participant M as 主行程
  participant W as 這份文件的 worker
  F->>M: export_pages { request, doc, pages, format }（不含路徑）
  M->>M: 驗證；作者禁止複製時拒絕
  M->>M: 系統的另存（純文字）或資料夾（圖片）對話框
  alt 使用者取消
    M-->>F: false
  else 選好位置
    loop 每一頁（背景工作，畫面的渲染優先）
      M->>W: GetPageText／RenderPng／RenderJpeg
      W-->>M: 文字／PNG／JPEG 位元組
      M->>M: 寫檔（圖片逐頁；文字最後一次寫出）
      M-->>F: 頻道：Progress { pagesDone, total }
    end
    M-->>F: true
  end
```

- **前端只說要匯出什麼**：
  - 文件、頁面（0 起算，最多 `LIMITS.maxExportPages`＝1,000 頁，不重複）、格式；
  - **不含路徑，也不經手檔案內容**。位置由主行程的系統對話框決定，內容由 worker 產生、主行程寫入。
- **對話框**（`src-tauri/src/file_dialog.rs`，與開啟對話框相同的做法）：
  - 純文字：`IFileSaveDialog`，建議檔名 `<原檔名>.txt`，已存在時由對話框本身詢問是否取代；
  - 圖片：`IFileOpenDialog` 的選擇資料夾模式；
  - 都有 `FOS_DONTADDTORECENT`：不加入 Windows 的「最近使用的項目」。
- **圖片的檔名**由主行程產生：`<原檔名>-p<頁碼>.png` 或 `.jpg`（頁碼從 1 起算）。資料夾中已有同名檔案時，以原生訊息方塊詢問「已經有 N 個同名的檔案。要覆寫嗎？」，選「否」就什麼都不寫。
- **寫檔**：先寫新的暫存檔 `<檔名>.tmp` 再改名，匯出中斷時不會留下半個檔案。
  - 資料夾中已經有同名的暫存檔時改用 `<檔名>.<n>.tmp`，不會覆寫使用者其他的檔案；
  - 圖片逐頁寫入，停止或失敗時已寫好的頁面保留；
  - 純文字在最後一次寫出，停止時不寫任何東西。
- **進度與停止**：進度走頻道；`cancel(request)` 在下一頁之前停止（與搜尋相同的做法）。每一頁都是渲染執行緒上的背景工作，畫面的渲染優先。

## 內容

- **純文字**：
  - 依頁序，每一行之後 `\r\n`，每一頁之後換頁字元 `\f`（與 `pdftotext` 相同）；
  - UTF-8，沒有 BOM；
  - 沿用 `get_page_text` 的清理規則（MVP-15：不含控制字元、雙向文字控制與零寬字元）；
  - 沒有文字層的頁面寫「（此頁沒有文字層）」。
- **PNG**：
  - worker 以 MuPDF 渲染並編碼（`RenderPng`），不套用檢視的旋轉，白色背景、沒有透明；
  - 72、150 或 300 dpi；
  - 超過點陣上限（`LIMITS.maxRasterPixels`）的頁面以較低的解析度匯出，與畫面的渲染相同；
  - 單一檔案最多 64 MB（`MAX_PNG_BYTES`），主行程收到時檢查 PNG 的簽名。
- **JPG**（#111）：
  - `mupdf` 繫結的影像編碼沒有 JPEG，所以 worker 以 [`jpeg-encoder`](https://github.com/vstroebel/jpeg-encoder)（`RenderJpeg`）編碼 MuPDF 渲染的點陣：
    - 純 Rust、沒有任何依賴；沒有開啟 `simd` 功能，整個套件 `forbid(unsafe_code)`；
    - 授權是 MIT 或 Apache-2.0，加上移植自 libjpeg 的部分為 IJG；`deny.toml` 只對這個套件允許 IJG。MuPDF 本身也附帶 IJG 授權的 libjpeg，發布時的第三方授權聲明要註明 Independent JPEG Group；
  - 品質固定 90：文字清楚，檔案比同一頁的 PNG 小；
  - 其他與 PNG 相同：72、150 或 300 dpi，白色背景，超過點陣上限時降低解析度；
  - 單一檔案最多 64 MB（`MAX_JPEG_BYTES`），主行程收到時檢查 JPEG 的開頭（`FF D8 FF`）；
  - worker 仍然不寫檔，只回傳位元組。

## 權限

作者禁止複製（MVP-19）時，匯出一併停用，比照 Acrobat 的「內容複製」：

- 「⋯」→「匯出…」停用並標示「作者不允許」；
- 主行程也會拒絕 `export_pages`。

## PDF 檔案：拆分（B2-06）

匯出對話框的另外兩種格式把頁面存成 PDF 檔案：選的頁面存成一個新檔案，或每幾頁存成一個檔案。做法、限制與測試見 [split.md](split.md)。與純文字和圖片的差異：頁面不是一頁一頁取回再寫出，而是 worker 直接把這些頁面寫成檔案（只能寫入的 handle），所以頁數上限是文件的頁數；加密的文件不能存成 PDF 檔案。

## 尚未處理

- **Word（.docx）**：需要 POC 與 ADR。
- 掃描頁的文字（OCR，B2-10）。

## 驗證

- Rust：
  - 對話框物件的選項（三種對話框都有 `FOS_DONTADDTORECENT`）；
  - 檔名、文字檔的格式；
  - 寫檔：整個檔案一次取代，不覆寫資料夾中同名的暫存檔；
  - worker 的 PNG（簽名、尺寸、比例上限）；
  - worker 的 JPG：以 MuPDF 解碼回來，尺寸正確，而且與畫面渲染的同一頁逐像素比較，平均差異很小（列的順序、色彩通道都正確）；
  - IPC 驗證：頁數上限、不重複、只接受 72／150／300 dpi、PNG 與 JPEG 的開頭與大小。
- 前端（`ExportDialog.test.tsx`）：純文字、PNG 與 JPG 的參數（解析度只用於圖片）、進度與停止、關閉系統對話框後仍可再試、頁碼不在文件中、禁止複製時停用。
- E2E（`tests/e2e/export.spec.ts`，以 UI Automation 回答系統的對話框）：
  - 純文字寫到另存的檔案；
  - 兩頁 PNG 寫到選的資料夾（簽名、寬度）；
  - 一頁 150 dpi 的 JPG：開頭正確，在 WebView 中可以開啟，1275 × 1650 像素；
  - 取消時什麼都不寫。
