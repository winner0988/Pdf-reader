# 頁碼、頁首頁尾與 Bates 編號（B3-16）

工作卡 [#304](https://github.com/winner0988/Pdf-reader/issues/304)；規格 §7「另存副本與頁面標記」。

這一部分提供將頁碼、Bates 編號、頁首頁尾畫在 PDF 副本頁面內容上的能力：包含 IPC 契約、前端介面與即時預覽、主行程暫存寫入保護，以及 worker 的 MuPDF 繪圖實作。批次處理多個檔案的頁碼與 Bates 跨檔連號見 [batch.md](batch.md)「頁碼與 Bates 編號」。

## 標記內容與規則

| 項目 | 規則 |
|---|---|
| 頁碼（`PageNumberMark`） | 格式 `Number`（「{n}」）、`Page`（「第 {n} 頁」）、`PageOfTotal`（「第 {n} 頁／共 {total} 頁」）、`Custom { prefix, suffix }`（各最多 100 個字元；空白照打的保留，控制字元與看不見的格式字元〔雙向文字控制、零寬字元〕拒絕）；起始號碼（≥ 1）；位置。 |
| Bates 編號（`BatesMark`） | 前綴（最多 32 個字元，規則與自訂頁碼的前後綴相同，例如 `ABC 000001` 的空白保留）、位數 1～12 位、起始號碼（≥ 0）；位置。依位數自動在左側補零（例如 `CASE-000001`）。起始號碼加上要編的頁數若超過位數能表示的上限（`start + count - 1 >= 10^digits`），拒絕處理且不寫出檔案。 |
| 頁首與頁尾（`HeaderFooterMarks`） | 六格（上左、上中、上右、下左、下中、下右），每格最多 100 個字元，不能換行或包含控制字元、看不見的格式字元；對話框送出前把連續的空白收成一個、去掉頭尾的空白。需要排版的文字（阿拉伯文、印度文字等）予以拒絕。 |
| 通用外觀與頁面範圍 | 字級 6～72 點、文字顏色（RGB）、離邊距離 0～144 點；頁面範圍：全部（`All`）、第一頁以外（`AllButFirst`）、自訂範圍（`Custom { pages }`，頁碼清單或區間，例如 `1-5, 8` 或 `1 - 3，5～6`，上限 10,000 頁）。一頁的文件選「第一頁以外」沒有頁面可標，worker 拒絕，對話框也停用確定按鈕。 |
| 位置衝突防護 | 六格位置中，任一格至多只能放置一樣標記（頁碼、Bates 編號、頁首頁尾其中之一）；同一格放兩樣東西時，前端對話框即時停用確定按鈕，契約與 worker 亦嚴格拒絕。 |

## 號碼算法

- **只算要標的頁面**：標記號碼依據實際納入編號的頁面計數。
- 第一個要標的頁面印出起始號碼（`start`），之後每標一頁加 1。
- `{total}` 代表最後一個要標的頁面之編號，即 `start + marked_count - 1`。
  - 例如：10 頁的文件，若選擇「第一頁以外」，則只有第 2 頁至第 10 頁（共 9 頁）被標記。若起始號碼為 1，則第 2 頁印「第 1 頁／共 9 頁」，第 10 頁印「第 9 頁／共 9 頁」。
- Bates 編號的末位號碼亦為 `start + marked_count - 1`，若此值 ≥ `10^digits` 即觸發溢位拒絕。

## 座標轉換與排版

- **畫成頁面內容，不是註解**：每一頁的 `/Contents` 被包裝成 `[q, 原本的內容…, Q, 新內容]`，確保原本內容的繪圖狀態（顏色、座標變換）不會外洩至頁面標記中。複製文字或搜尋時均能找到頁碼與 Bates 字串。
- **XObject 與獨立資源**：各格標記以獨立的 Form XObject（命名為 `/PdfReaderPageMarks_TL`、`/PdfReaderPageMarks_TC`、`/PdfReaderPageMarks_TR`、`/PdfReaderPageMarks_BL`、`/PdfReaderPageMarks_BC`、`/PdfReaderPageMarks_BR`）儲存於頁面的 `/Resources` 字典中。若頁面的 Resources 為繼承而來，先深拷貝一份予該頁獨立持有。
- **CropBox 與旋轉處理**：利用 MuPDF 頁面的可見範圍（CropBox）計算上中下與左中右的座標。透過 MuPDF 的 `page_ctm` 逆矩陣轉換（`inverse`），配合 Form XObject 內部的 `Matrix::new_scale(1.0, -1.0)` 翻轉，使得即使頁面設有 `/Rotate 90` 或 `/Rotate 270`，印在頁面上的標記字樣依然與使用者「顯示時」的方向一致（保持水平端正）。
- **字型子集與 ToUnicode**：文字採用 worker 內建的 Droid Sans Fallback，將用到的字元抽取子集並附帶 ToUnicode 映射表嵌入 PDF，不讀取系統字型，確保離線環境下不同閱讀器皆能正常檢視與搜尋。
- **整份副本只嵌入一個子集**：開始前先收集所有標記可能用到的字元（頁首頁尾的字、頁碼格式與前後綴、Bates 前綴，加上 0～9），切出一個子集寫進副本一次；之後每一頁的頁碼與 Bates 編號都只是一個引用這個字型的小 Form XObject。若每一頁各嵌一個子集，數千頁的文件會多出數千個字型、也要切數千次字型（`text_box::line_font`、`line_form_in`）。

## 流程與安全拒絕

1. **原檔與開著的文件完全不變**：
   - 副本產生流程在記憶體中將目前文件（含編輯）重開成第二份實例，僅在副本上加上 Form XObject。
   - 主行程（Tauri）僅將唯寫的暫存檔 handle 交給 worker，worker 寫畢後主行程驗證位元組長度、`%PDF-` 開頭與 `%%EOF` 結尾，再 atomic 取代為目標檔名。
   - 不覆寫原檔（大寫拼寫或連結亦會被阻擋），且不加入「最近開啟」歷程。
2. **拒絕條件**：
   - 加密文件拒絕：worker 開啟後不保留密碼，無法將副本重新加密。
   - 數位簽章說明：有簽章的文件允許匯出副本，但對話框與架構說明清楚標示副本的數位簽章將不成立。

## 驗證

- **IPC 契約**（`crates/ipc_contract`）：
  - 欄位範圍與限制（字級、邊距、字數限制、位數上限、頁數上限）；
  - 同一格位置衝突拒絕；
  - Bates 位數溢位拒絕；
  - Postcard 二進位序列化往返正確性。
- **worker 單元測試**（`crates/pdf_worker/src/engine/page_marks.rs`）：
  - 四種頁碼格式與 `{total}` 計算正確性；
  - 全部、第一頁以外、自訂範圍三種範圍及越界檢驗；
  - Bates 補零與溢位拒絕（不寫出任何檔案）；
  - 六格位置驗證（透過 `to_text_page` 擷取字元四角邊界確認落在各三分之一區塊）；
  - 旋轉頁面標記端正（`benign/rotated-page.pdf` 渲染後文字寬大於高）；
  - 副本沒有註解（`page_annotations` 為空）；
  - 原檔與開著的文件位元組未改變。
- **worker 實體沙盒測試**（`crates/pdf_worker/tests/page_marks.rs`）：
  - 在真實 Windows 限制 token 沙盒環境中執行 `pdf_worker.exe`；
  - 驗證每一頁產生的頁碼、Bates 編號與頁首；
  - 驗證加密文件與溢位拒絕時 worker 不會 crash 並繼續提供後續服務。
- **前端測試**（`src/features/saving/pageMarks.test.ts` & `PageMarksDialog.test.tsx`）：
  - 純邏輯單元測試（範圍解析、格式化、預覽文字計算、位置衝突檢測）；
  - React 元件測試（預覽更新、位置衝突停用確定按鈕、溢位停用確定按鈕、送出流程）。
- **E2E 測試**（`tests/e2e/page-marks.spec.ts`，交給 CI 執行）：
  - 完整 UI 流程測試：開檔 → 開啟對話框 → 設定頁碼與 Bates → 存檔 → 開啟副本 → 搜尋「共 10 頁」與「0000」各有 10 筆（每一頁一筆）、「第 7 頁」與「000010」各 1 筆 → 原檔 SHA-256 不變。
  - 轉過的頁面方向由 worker 單元測試檢查（E2E 看不到字的方向）。
