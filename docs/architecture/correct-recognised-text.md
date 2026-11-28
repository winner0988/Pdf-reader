# 修正辨識的文字（B7-12）

**做法決定**：[ADR 0036](../adr/0036-edit-existing-text.md)（只改隱形文字層，不碰畫面上的任何字）與 [ADR 0028](../adr/0028-searchable-pdf-text-layer.md)（無字形隱藏字型 `PdfReaderOcrFont`）。

針對已經寫入 PDF 的 `3 Tr`（invisible render mode）文字層進行單詞修正：只更新搜尋、複製、全文索引與匯出讀取到的隱藏文字，頁面的視覺渲染與非 `3 Tr` 文字保持不變。

## 資料流

1. **掃描可改的詞（`get_recognised_words` → worker `GetRecognisedWords`）**
   - 前端只在「修正辨識的文字」模式開啟後，依目前頁面查詢 `get_recognised_words({ doc, pageIndex })`。
   - Worker 的 `crates/pdf_worker/src/engine/recognised_text.rs` 以共用函式 `filter_options`（`recurse: false`、`instance_forms: true`）呼叫 `PdfPage::filter_text`（列字時 `no_update: true`），在每個 `Do` 處以頁面空間座標（含 `cm` 與 `/Matrix` 變換，同一個 Form 畫兩次就是兩個字）走訪字元；只收集 `glyph.render_mode() == Some(TextRenderMode::Invisible)`（嚴格為 `3 Tr`，不用 `is_invisible()`），排除僅靠 alpha 0、繪圖顏色相同或純裁切 `7 Tr` 隱藏的文字。列字時 `instance_forms: true` 產生的未引用 Form 副本不會掛進頁面資源或內容串流，會在之後完整重寫存檔（`save_checked` 的 `set_garbage(true)`）時丟掉（已簽章的文件在掃描前即拒絕）。
   - Quad 的 8 個座標（`ul`、`ur`、`ll`、`lr`）或組詞後最終 `rect`／`quad` 只要非有限值或絕對值超過 `MAX_PAGE_SIDE_PT` 就不列出（頁外字形當作分隔略過），避免單一頁外 `3 Tr` 字觸發主行程合約拒絕而關掉 worker。
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
     - 透過 `remove_tr3_glyphs` 以同一組選項 `filter_options(false)`（`recurse: false`、`instance_forms: true`、`no_update: false`）移除目標字元索引中符合 `glyph.render_mode() == Some(TextRenderMode::Invisible)` 的字形；兩次走訪使用完全相同的走訪順序與頁面座標，`instance_forms: true` 會替目標頁抽換出頁面專用的 Form XObject 副本（同一頁畫兩次的 Form 也各自獨立），不影響同文件其他頁面。
     - 確保頁面資源字典掛載共用的隱藏字型 `/PdfReaderOcrFont`（重用文件既有的 `GlyphLessFont` 物件，避免每改一個字重複寫入 160 KB 字型），在 `page_to_pdf`（考慮 `/MediaBox` 原點與頁面 `/Rotate`）座標空間加上孤立的 `q ... Q` 與 `Tz` 縮放後的 `3 Tr` UTF-16BE 文字串流。
     - 立即清除該頁文字與搜尋快取（`invalidate_page_text_cache`），使 `Ctrl+F` 與選取在存檔前就立刻反映新文字；每次修正皆為單一次可 `Ctrl+Z`／`Ctrl+Y` 復原的編輯，存檔走既有安全重寫流程。

## 相關限制與考量

- **詞切分邊界**：空白或間距超過門檻（沿基線位移、垂直偏移或方向餘弦）會拆成獨立的詞；目前一次修正一個分割後的 `RecognisedWord`。
- **Form XObject 與頁外文字**：列字與移除共用同一組選項（`recurse: false`、`instance_forms: true`），Form 內的字使用頁面座標（含 `cm` 與 `/Matrix`），同一個 Form 畫兩次會列出兩個獨立的詞；列字（`no_update: true`）產生的未引用副本由存檔時 `set_garbage(true)` 丟掉（有簽章文件在掃描前就已拒絕），修改（`no_update: false`）只影響目標頁面與目標 `Do` 副本。座標非有限值或超出 `MAX_PAGE_SIDE_PT` 的頁外 `3 Tr` 字不列出。

## 測試與安全檢查

- **契約與主行程**：`ipc_contract::validate` 測試空白／過長／控制字元／字數上限與 `RecognisedWordsPage` 雙模式序列化；`src-tauri/src/documents.rs` 測來回編輯、權限檢查與已簽章拒絕。
- **Worker 單元與整合測試（`recognised_text.rs`、`crates/pdf_worker/tests/correct_recognised_text.rs`）**：包含語料庫 `benign/recognised-text-layer.pdf`、`qpdf --check`、像素完全不變檢查、存檔前立即搜尋生效、`0 Tr`／`7 Tr`／alpha 0 負向對照（`recognised_text.rs` 單元測試把 `(0..100_000)` 全部序號傳給 `remove_tr3_glyphs` 以守住移除步驟的 `render_mode() == Some(TextRenderMode::Invisible)` 檢查，並保留 `control_using_is_invisible_removes_alpha_zero_text` 驗證 `is_invisible()` 會誤刪 alpha-0 文字）、旋轉 90° 搭配非零 `/MediaBox`、Form XObject（`Do` 後接頁面字、`cm` 與 `/Matrix` 變換、同 Form 畫兩次、僅列字存檔保留共用 `/Fm0`、跨頁隔離）、列字不改動 Type 3 字型資源、頁外 `3 Tr` 字略過且保持 worker 可用、重用單一 `GlyphLessFont`、以及 `Undo`/`Redo`。
- **Fuzz 與 E2E**：納入 `fuzz/fuzz_targets/edit_document.rs` 覆蓋（36/36 `WorkerEdit`）；前端元件測試見 `src/features/ocr/CorrectRecognisedText.test.tsx`，E2E 見 `tests/e2e/correct-recognised-text.spec.ts`。
