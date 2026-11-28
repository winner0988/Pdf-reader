# 修正辨識的文字（B7-12）

**做法決定**：[ADR 0036](../adr/0036-edit-existing-text.md)（只改隱形文字層，不碰畫面上的任何字）與 [ADR 0028](../adr/0028-searchable-pdf-text-layer.md)（無字形隱藏字型 `PdfReaderOcrFont`）。

針對已經寫入 PDF 的 `3 Tr`（invisible render mode）文字層進行單詞修正：只更新搜尋、複製、全文索引與匯出讀取到的隱藏文字，頁面的視覺渲染與非 `3 Tr` 文字保持不變。

## 資料流

1. **掃描可改的詞（`get_recognised_words` → worker `GetRecognisedWords`）**
   - 前端只在「修正辨識的文字」模式開啟後，依目前頁面查詢 `get_recognised_words({ doc, pageIndex })`。
   - Worker 的 `crates/pdf_worker/src/engine/recognised_text.rs` 以 `PdfPage::filter_text`（`no_update: true`、`recurse: true`、`instance_forms: false`）走訪字元；只收集 `glyph.render_mode() == Some(TextRenderMode::Invisible)`（嚴格為 `3 Tr`，不用 `is_invisible()`），排除僅靠 alpha 0、繪圖顏色相同或純裁切 `7 Tr` 隱藏的文字。
   - 依空白字元與字距／基線臨界值分割成視覺閱讀順序的 `RecognisedWord` 清單（上限 `MAX_RECOGNISED_WORDS_PER_PAGE = 20000`），計算每個詞的頁面空間矩形與四邊形。
   - **回傳序列化（`RecognisedWordsPage`）**：worker 與主行程之間的 postcard frame 不支援內部標籤列舉，前端 JSON 需要 `{ status: "usable", words }` / `{ status: "unusable", reason }` 的標籤格式；因此依 `serializer.is_human_readable()` 分別序列化為 `Compact`（postcard）與 `Tagged`（JSON）。

2. **不直接開放修改的情況（`RecognisedWordsUnusable`）**
   - `signed`：文件有數位簽章，任何改動都會影響簽章驗證。
   - `notAllowed`：作者透過 `/P` 權限禁止修改文件。
   - `memoryOcrOnly`：目前頁面的辨識結果僅來自開啟期間的記憶體 OCR、尚未寫入 PDF `3 Tr` 內容串流（提示使用者先另存為可搜尋的 PDF 再打開副本修正）。
   - `noTextLayer`：頁面沒有可修正的 `3 Tr` 隱藏文字。
   - `mixedRenderModes`：同一個連續文字區段把 `3 Tr` 與裁剪文字模式（`4..=7 Tr`）混用時，為避免移除誤傷頁面裁切效果，整頁拒絕編輯。

3. **套用修改、復原與快取（`Edit::CorrectRecognisedText`）**
   - 前端延用 [InlineTextEditor](../../src/features/annotations/InlineTextEditor.tsx)（以 `plainText` 模式關閉字型／樣式工具列，並不做 `withoutAnnotations` 畫面遮罩）在詞的位置就地開啟輸入框，明確標示「只改搜尋與複製用的字，畫面不會改變」。
   - 送出的 `Edit::CorrectRecognisedText { page, word, expected, text }` 在主行程驗證長度（`MAX_RECOGNISED_WORD_BYTES = 4096`）、不得空白與不得含控制字元／雙向覆寫字元後轉為 `WorkerEdit::CorrectRecognisedText`。
   - Worker 重新掃描目標頁並比對 `word` 與 `expected` 字串一致後：
     - 用 `filter_text` 移除該詞原本所屬的內容串流（或目標頁面／Form XObject）中對應的字元索引；若 `3 Tr` 詞位於共用的 Form XObject，先複製該 Form XObject 成頁面專用副本再修改，避免影響其他頁面。
     - 確保頁面（或該 Form XObject）資源字典掛載共用的隱藏字型 `/PdfReaderOcrFont`（重用文件既有的 `GlyphLessFont` 物件，避免每改一個字重複寫入 160 KB 字型），在 `page_to_pdf`（考慮 `/MediaBox` 原點與頁面 `/Rotate`）座標空間加上孤立的 `q ... Q` 與 `Tz` 縮放後的 `3 Tr` UTF-16BE 文字串流。
     - 立即清除該頁文字與搜尋快取（`invalidate_page_text_cache`），使 `Ctrl+F` 與選取在存檔前就立刻反映新文字；每次修正皆為單一次可 `Ctrl+Z`／`Ctrl+Y` 復原的編輯，存檔走既有安全重寫流程。

## 相關限制與考量

- **詞切分邊界**：空白或間距超過門檻（沿基線位移、垂直偏移或方向餘弦）會拆成獨立的詞；目前一次修正一個分割後的 `RecognisedWord`。
- **共用的 Form XObject**：先淺複製（clone）只給該頁參照的新 Form XObject 才修改，同文件其他引用舊 Form XObject 的頁面保持原狀。

## 測試與安全檢查

- **契約與主行程**：`ipc_contract::validate` 測試空白／過長／控制字元／字數上限與 `RecognisedWordsPage` 雙模式序列化；`src-tauri/src/documents.rs` 測來回編輯、權限檢查與已簽章拒絕。
- **Worker 整合測試（`crates/pdf_worker/tests/correct_recognised_text.rs`）**：包含語料庫 `benign/recognised-text-layer.pdf`、`qpdf --check`、像素完全不變檢查、存檔前立即搜尋生效、`0 Tr`／`7 Tr`／alpha 0 負向對照（`control_using_is_invisible_removes_alpha_zero_text` 驗證 `is_invisible()` 會誤刪 alpha-0 文字）、旋轉 90° 搭配非零 `/MediaBox`、跨頁共用 Form XObject 隔離、重用單一 `GlyphLessFont`、以及 `Undo`/`Redo`。
- **Fuzz 與 E2E**：納入 `fuzz/fuzz_targets/edit_document.rs` 覆蓋（36/36 `WorkerEdit`）；前端元件測試見 `src/features/ocr/CorrectRecognisedText.test.tsx`，E2E 見 `tests/e2e/correct-recognised-text.spec.ts`。
