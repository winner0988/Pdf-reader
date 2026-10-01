# 註解：螢光筆與文字附註（B2-07）

工作卡 [#96](https://github.com/winner0988/Pdf-reader/issues/96)；規格 §6「註解標記：螢光筆」。編輯的共通做法（指令、復原、存檔、崩潰復原）見 [ADR 0013](../adr/0013-editing-and-saving.md)、[page-management.md](page-management.md)、[saving.md](saving.md) 與 [crash-recovery.md](crash-recovery.md)。

這份文件先說明 worker 與主行程的部分；畫面（選取文字後的「螢光筆」、放置附註、選取既有的註解）在下一個 PR。

## 註解是標準的 PDF 註解

| app 的動作 | PDF 中的註解 | 說明 |
|---|---|---|
| 螢光筆 | `Highlight`，`/QuadPoints` 是選取的文字的四邊形（每行一個），`/C` 是顏色 | 四種顏色：黃、綠、藍、粉紅 |
| 文字附註 | `Text`（附註圖示，20 × 20 pt），`/Contents` 是附註的文字 | MuPDF 另外加上一個彈出視窗（`Popup`） |

- 每個新註解都有外觀串流（`/AP`，MuPDF 產生），其他閱讀器也看得到。
- **不寫入任何可識別使用者的資訊**：MuPDF 建立註解時不寫作者（`/T`）、建立與修改時間（`/CreationDate`、`/M`）或唯一名稱（`/NM`），app 也不加。worker 的測試檢查這幾個欄位都不存在。

## 編輯指令

都是 `apply_edit` 的 `Edit`，與頁面編輯相同：主行程先驗證，在文件自己的 worker 中套用，記入編輯歷史（可以復原、重做）與崩潰復原日誌，存檔時才寫入檔案。

| `Edit` | 內容 | 驗證 |
|---|---|---|
| `addHighlight { marks, color }` | 標示螢光筆：`marks` 的每一頁（`{ page, quads }`）各一個 `Highlight` 註解。選取的文字跨頁時仍是一個編輯，一次復原全部取消 | 1 到 `LIMITS.maxHighlightPages`（100）頁，頁碼不重複，每頁至少一個四邊形，全部最多 `LIMITS.maxAnnotationQuads`（1,000）個，座標都是有限值且不超過頁面大小的上限；有一頁不符合時，任何一頁都不改變 |
| `addNote { page, at, text }` | 在 `at` 放一個附註 | 文字見下方 |
| `deleteAnnotation { page, annotation }` | 移除一個註解（app 加的，或文件原有的） | 註解必須在那一頁 |
| `setHighlightColor { page, annotation, color }` | 改變螢光筆的顏色 | 只能是螢光筆 |
| `setNoteText { page, annotation, text }` | 改變附註的文字 | 只能是附註；文字見下方 |

- 座標是頁面空間（PDF 點，頁面左上角為原點，y 向下），與文字選取、搜尋結果的四邊形相同；worker 交給 MuPDF 換算成 PDF 的座標。
- **附註的文字**：不可以是空的或只有空白，最多 `LIMITS.maxNoteTextBytes`（4,096 bytes，UTF-8）；除了換行（`\n`）不能有控制字元，也不能有雙向控制字元、零寬字元等看不見的格式字元。使用者自己的空白保留。
- **權限**（MVP-19）：`/P` 的第 6 位元（註解）沒有設定時，主行程拒絕所有註解的編輯；頁面編輯仍依第 4、11 位元。以擁有者密碼開啟時全部允許（#88）。
- 頁碼不存在、註解不在那一頁、類型不符時，什麼都不改變。

## 註解的識別碼

- `AnnotationId` 是註解在文件中的物件編號。
- 編輯歷史重做時（復原、worker 崩潰、崩潰復原）從同一個檔案依序套用同樣的編輯，新註解得到同樣的編號，所以之後的編輯（改顏色、刪除）仍然指向同一個註解。
- 存檔後檔案重寫、物件重新編號；編輯歷史也從存檔後的檔案重新開始（ADR 0013），前端重新取得註解清單。

## 列出頁面的註解（`get_page_annotations`）

- 回傳 `PageAnnotation[]`：`id`、`kind`（`highlight`、`note`、`other`）、`rect`（顯示的範圍，頁面空間）、`color`（是 app 的四種顏色之一時）、`text`（附註的文字）。
- 不列出：連結（見 [links.md](links.md)）、表單欄位、彈出視窗。其他種類（手繪、印章、方框等，`other`）可以選取並刪除。
- 文件原有的註解照常由 MuPDF 渲染在頁面上。
- worker 的輸出一律當成不可信任的資料：
  - 附註的文字逐行清理成顯示用的文字（控制字元與連續空白變成一個空白，看不見的格式字元去掉，與目錄標題相同），最多 4,096 bytes；
  - 範圍不是有限值或超過頁面大小上限的註解不列出；
  - 每頁最多 `LIMITS.maxAnnotationsPerPage`（2,000）個；
  - 主行程再以 `validate` 檢查一次，並確認編號不重複、頁碼相符。

## 移除註解不留下內容

- MuPDF 從頁面的 `/Annots` 移除註解與它的彈出視窗。
- 其他還指向它們的地方（另一個註解的回覆 `/IRT`、彈出視窗的 `/Parent`、結構樹的 `/OBJR`）改成 `null`（`unlink.rs`，與刪除頁面相同，見 [page-management.md](page-management.md)）。
- 完整重寫的存檔就不會再包含它們，例如附註的文字。

## 測試

- worker（`crates/pdf_worker/src/engine.rs`）：
  - 列出註解，不含連結、表單欄位與彈出視窗；
  - 新的螢光筆與附註存檔後仍是標準註解，有 `/QuadPoints` 與外觀，沒有作者、時間與唯一名稱；
  - 改顏色、改文字、類型不符時拒絕；
  - 移除附註時一併移除彈出視窗，回覆不再指向它，存檔後檔案中沒有它的文字。
  - 負向對照：加上作者、或不切斷指向，對應的測試會失敗。
- 主行程（`src-tauri/src/documents.rs`）：新增、改變、移除、復原後編號不變；不存在的註解、頁碼與不是文字的附註被拒絕；作者不允許註解的文件（RC4 語料，`/P -44`）拒絕註解的編輯。
- 合約（`crates/ipc_contract`）：註解編輯與 worker 回傳的註解的驗證；附註文字的清理。
