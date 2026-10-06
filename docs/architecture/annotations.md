# 註解：螢光筆、文字附註、手繪線條與印章（B2-07、B2-08）

工作卡 [#96](https://github.com/winner0988/Pdf-reader/issues/96)（螢光筆與附註）、[#97](https://github.com/winner0988/Pdf-reader/issues/97)（手繪線條與印章）；規格 §6「註解標記：螢光筆」。編輯的共通做法（指令、復原、存檔、崩潰復原）見 [ADR 0013](../adr/0013-editing-and-saving.md)、[page-management.md](page-management.md)、[saving.md](saving.md) 與 [crash-recovery.md](crash-recovery.md)。

worker 與主行程的做法在前面各節；畫面（選取文字後的「螢光筆」、放置附註、選取與變更既有的註解）見「畫面」。

## 註解是標準的 PDF 註解

| app 的動作 | PDF 中的註解 | 說明 |
|---|---|---|
| 螢光筆 | `Highlight`，`/QuadPoints` 是選取的文字的四邊形（每行一個），`/C` 是顏色 | 四種顏色：黃、綠、藍、粉紅 |
| 文字附註 | `Text`（附註圖示，20 × 20 pt），`/Contents` 是附註的文字 | MuPDF 另外加上一個彈出視窗（`Popup`） |
| 手繪線條（B2-08） | `Ink`，`/InkList` 是每一筆的點（PDF 座標），`/C` 是顏色，`/BS` 的 `/W` 是粗細 | 四種顏色（黑、紅、藍、綠）、三種粗細（1、2.5、5 pt）；線條是圓頭圓角，只有一個點的一筆是一個圓點 |
| 標準印章（B2-08） | `Stamp`，`/Name` 是標準名稱（`Approved`、`NotApproved`、`Draft`、`Final`、`Confidential`、`ForComment`、`AsIs`、`TopSecret`），`/C` 是顏色 | MuPDF 產生外觀：英文字、細邊框，略微傾斜，與其他閱讀器的同名印章相同；大小固定 190 : 50 的比例 |

- 每個新註解都有外觀串流（`/AP`，MuPDF 產生），其他閱讀器也看得到。
- **不寫入任何可識別使用者的資訊**：MuPDF 建立註解時不寫作者（`/T`）、建立與修改時間（`/CreationDate`、`/M`）或唯一名稱（`/NM`），app 也不加。worker 的測試檢查這幾個欄位都不存在。

## 自訂圖片印章：worker 的部分（B2-08）

使用者選的圖片是不受信任的輸入，只在沙盒的 worker 裡解碼；選圖片的對話框、檔案路徑與按鈕在後面的 PR，這裡是 worker 這一端。

- **`PrepareStampImage`**：主行程把使用者選的檔案以**唯讀 handle**交給 worker（worker 拿不到路徑），worker 回 `StampImage`：一個 PNG，裡面**只有像素**。
  - 只收 PNG 與 JPEG（看開頭的位元組；GIF、BMP、TIFF、JPEG 2000 等一律拒絕，少一些解碼器就少一些攻擊面）；檔案最多 16 MiB（`MAX_STAMP_SOURCE_BYTES`），每邊最多 8,192、全部最多 16 Mpx，**在解碼任何像素之前**就檢查（PNG 先讀自己的標頭、MuPDF 再讀一次；JPEG 由 MuPDF 讀標頭）；
  - 殘缺的 PNG（沒有像素資料或結束區塊）拒絕，不當成空白圖片；
  - 解碼成像素之後只留灰階或 RGB（有無透明都可以），縮小到每邊最多 `MAX_STAMP_SIDE_PX`（1,024，每次減半）、再編碼成 PNG；編出來超過 `MAX_STAMP_PNG_BYTES`（1 MiB，雜訊一類壓不下去的圖）就再減半，直到夠小；這個大小也是為了讓崩潰復原的日誌（4 MiB，B2-13）放得下一張圖片加上其他編輯；
  - **不留任何中繼資料**：EXIF（GPS 位置、相機型號、拍攝時間）、PNG 的文字區塊（`tEXt`）、`eXIf`、色彩描述檔等都隨檔案一起丟掉，輸出只有 `IHDR`、`pHYs`、`IDAT`、`IEND`。已知限制：JPEG 的 EXIF 方向不套用（照片會是相機拍的方向，要先轉好再選）。
- **`AddImageStamp { page, rect, png }`**（`WorkerEdit`，主行程用）：`png` 是上面做出來的檔案，worker 再檢查一次（簽名、標頭、大小）、再解碼一次、用像素做成一個新的圖片物件，**不用原來的位元組**（MuPDF 收到 JPEG 檔會原樣放進 PDF，EXIF 也跟著進去；測試的對照組證明這點）。`Stamp` 註解的外觀是一個蓋住單位正方形的表單，畫這張圖，由讀者對應到 `rect`；`/Name` 是 `Picture`（不是標準印章，MuPDF 不會重畫外觀）。
- 測試：`crates/pdf_worker/src/engine/stamp_image.rs`（解碼與重新編碼、EXIF 與 PNG 區塊沒有了、透明保留、存檔的 PDF 沒有任何一個私密字串、對照組：原樣放進去的 JPEG 有；畫出來的像素是圖片的顏色；不是 PNG／JPEG、殘缺、太大、標頭謊稱的大小都拒絕；大圖縮小、雜訊壓到上限內）與 `crates/pdf_worker/tests/stamp_image.rs`（經過真正的沙盒與唯讀 handle，壞圖片拒絕之後 worker 仍可用）。語料：`tests/corpus/images/`（見 [README](../../tests/corpus/README.md)）。

## 編輯指令

都是 `apply_edit` 的 `Edit`，與頁面編輯相同：主行程先驗證，在文件自己的 worker 中套用，記入編輯歷史（可以復原、重做）與崩潰復原日誌，存檔時才寫入檔案。

| `Edit` | 內容 | 驗證 |
|---|---|---|
| `addHighlight { marks, color }` | 標示螢光筆：`marks` 的每一頁（`{ page, quads }`）各一個 `Highlight` 註解。選取的文字跨頁時仍是一個編輯，一次復原全部取消 | 1 到 `LIMITS.maxHighlightPages`（100）頁，頁碼不重複，每頁至少一個四邊形，全部最多 `LIMITS.maxAnnotationQuads`（1,000）個，座標都是有限值且不超過頁面大小的上限；有一頁不符合時，任何一頁都不改變 |
| `addNote { page, at, text }` | 在 `at` 放一個附註 | 文字見下方 |
| `deleteAnnotation { page, annotation }` | 移除一個註解（app 加的，或文件原有的） | 註解必須在那一頁 |
| `setHighlightColor { page, annotation, color }` | 改變螢光筆的顏色 | 只能是螢光筆 |
| `setNoteText { page, annotation, text }` | 改變附註的文字 | 只能是附註；文字見下方 |
| `addInk { page, strokes, color, width }` | 手繪：`strokes` 的每一筆（頁面空間的點）一起成為一個 `Ink` 註解，所以一次復原取消整張圖 | 至少一筆、每筆至少一個點；最多 `LIMITS.maxInkStrokes`（256）筆、全部最多 `LIMITS.maxInkPoints`（20,000）個點，座標都是有限值且不超過頁面大小的上限 |
| `addStamp { page, rect, stamp }` | 在 `rect` 蓋上標準印章 | `rect` 是正的，每邊至少 `LIMITS.minAnnotationSidePt`（8 pt），座標都是有限值 |
| `setAnnotationRect { page, annotation, rect }` | 移動並縮放手繪或印章，`rect` 是列出的範圍（見下方）的新位置 | 同上；只能是手繪或印章 |

- 座標是頁面空間（PDF 點，頁面左上角為原點，y 向下），與文字選取、搜尋結果的四邊形相同；worker 交給 MuPDF 換算成 PDF 的座標。
- **移動與縮放**（`setAnnotationRect`）：印章交給 MuPDF 改 `/Rect`，外觀依印章固有的比例縮放並置中（列出的範圍是縮放後的）；手繪把每個點重新排進新的範圍：範圍是點的外框加上 MuPDF 在線條四周留的邊（線寬 + 6 pt，讓細線、圓點也點得到），這個邊保持不變，所以只移動時每個點都平移同樣的距離，線條粗細也不變。沒有範圍的軸（水平或垂直的直線）的點放在新範圍的中央；已有超過 `LIMITS.maxInkPoints` 個點的手繪（文件原有的）不移動。
- **附註的文字**：不可以是空的或只有空白，最多 `LIMITS.maxNoteTextBytes`（4,096 bytes，UTF-8）；除了換行（`\n`）不能有控制字元，也不能有雙向控制字元、零寬字元等看不見的格式字元。使用者自己的空白保留。
- **權限**（MVP-19）：`/P` 的第 6 位元（註解）沒有設定時，主行程拒絕所有註解的編輯；頁面編輯仍依第 4、11 位元。以擁有者密碼開啟時全部允許（#88）。
- 頁碼不存在、註解不在那一頁、類型不符時，什麼都不改變。

## 註解的識別碼

- `AnnotationId` 是註解在文件中的物件編號。
- 編輯歷史重做時（復原、worker 崩潰、崩潰復原）從同一個檔案依序套用同樣的編輯，新註解得到同樣的編號，所以之後的編輯（改顏色、刪除）仍然指向同一個註解。
- 存檔不改變編號：存檔只丟掉沒有用到的物件（`PdfWriteOptions` 的 garbage 等級 1），不會把物件重新編號（等級 2 以上才會）。所以 worker 重新開啟存好的檔案（崩潰後，或存檔後的復原）時，前端已經知道的編號仍然對得上；worker 的測試（`annotation_numbers_stay_the_same_through_saving_and_opening_again`）固定這一點，改成會重新編號的選項它就會失敗。
- 註解被刪除後，它的編號不會再列出；前端也不會用到它。

## 列出頁面的註解（`get_page_annotations`）

- 回傳 `PageAnnotation[]`：`id`、`kind`（`highlight`、`note`、`ink`、`stamp`、`other`）、`rect`（顯示的範圍，頁面空間；手繪含線條四周的邊）、`color`（螢光筆是 app 的四種顏色之一時）、`text`（附註的文字）。
- 不列出：連結（見 [links.md](links.md)）、表單欄位、彈出視窗。手繪與印章（含文件原有的）可以移動與縮放，其他種類（方框等，`other`）只能選取並刪除。
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

## 畫面

畫面的行為與文字見 [screen-map.md](../ux/screen-map.md)「註解（B2-07）」。程式在 `src/features/annotations/`。

- **標示螢光筆**：先選取文字（MVP-15），再從右鍵功能表的「螢光筆」選顏色，或按工具列的螢光筆按鈕（用上次的顏色，一開始是黃色）。
  - 選取的文字在每一頁各有一組四邊形（頁面空間，與文字選取、搜尋相同）；`DocumentView` 的 `highlightMarks()` 向文字來源取得，組成一個 `addHighlight`：跨頁也只是一個編輯，一次復原全部取消。
  - 超過 `LIMITS.maxHighlightPages`（100）頁或 `LIMITS.maxAnnotationQuads`（1,000）個四邊形時不送出，狀態列說明請分段標示。
  - 編輯之後文件換了新的編號，選取自然清除，只剩 MuPDF 畫在頁面上的螢光筆。
- **新增附註**：在頁面上按右鍵，選「在這裡新增附註…」；位置是按右鍵的地方（`DocumentView` 的 `pageAt()`，頁面以外的地方不能放），內容在對話框輸入。
- **選取既有的註解**：每個註解在頁面上有一個透明的輪廓按鈕（可用 `Tab` 與螢幕閱讀器操作，標籤是 `annotationLabel`），也可以在註解上點一下。
  - 輪廓讓滑鼠通過：滑鼠點擊是由 `DocumentView` 依位置找出被點的註解（`annotationAt`），所以螢光筆下面的文字仍然可以選取。按下去後移動超過 4 px 是在選取文字，不是點選。
  - 選取的註解旁出現一個小工具列：螢光筆有四種顏色與刪除，附註有文字、編輯與刪除，其他種類只有刪除。`Delete` 刪除，`Esc` 放開。
  - 編輯讓選取的註解不見時（例如復原），自動放開。
- **作者不允許註解**（`/P` 第 6 位元）：工具列的螢光筆與右鍵的項目停用，右鍵的「新增附註」標示「作者不允許」；註解仍然顯示，也可以選取，但工具列的按鈕都停用，`Delete` 不作用。
- **文字的處理**：使用者輸入的附註文字先由 `noteText` 整理成主行程接受的形式：換行統一成 LF、Tab 變成四個空白、控制字元與看不見的格式字元去掉、頭尾空白去掉；空的或超過 `LIMITS.maxNoteTextBytes`（以 UTF-8 計）時對話框說明，不送出。主行程仍會再檢查一次。

## 測試

- 前端（`src/features/annotations/`）：
  - `model.test.ts`：附註文字的整理與長度、找出位置上的註解、名稱；
  - `source.test.ts`：每頁只問一次、換文件重問、失敗後再問；
  - `Annotations.test.tsx`：右鍵標示並選顏色、工具列按鈕用上次的顏色、作者不允許時不提供、太多時的說明、新增附註（位置與內容）、空的與太長的附註、頁面以外不能放、以鍵盤或點擊選取註解並改顏色與刪除、改附註、作者不允許時只能看。
- E2E（`tests/e2e/annotations.spec.ts`）：選取 Privacy 標示螢光筆，輪廓覆蓋這個字；復原與重做；改顏色；另存新檔，檔案中有標準的 `Highlight` 與 `QuadPoints`、沒有作者與建立時間；重新開啟後仍在，可以刪除。另一個測試新增附註並改內容。

- worker（`crates/pdf_worker/src/engine.rs`）：
  - 列出註解，不含連結、表單欄位與彈出視窗；
  - 新的螢光筆與附註存檔後仍是標準註解，有 `/QuadPoints` 與外觀，沒有作者、時間與唯一名稱；
  - 改顏色、改文字、類型不符時拒絕；
  - 移除附註時一併移除彈出視窗，回覆不再指向它，存檔後檔案中沒有它的文字；
  - 存檔並重新開啟後，註解的編號不變；
  - 一次標示多頁時，有一頁不符就什麼都不改變；
  - 手繪（B2-08）：存檔、重新開啟後是標準的 `Ink`，筆畫的點（PDF 座標）、顏色、粗細都在，有外觀，沒有作者與時間，範圍是點加上線寬與邊；一筆只有一個點是圓點；沒有筆畫、有空的一筆、頁碼不存在時拒絕；
  - 八種標準印章：`/Name` 正確、外觀有對應的英文字、有顏色，沒有作者與時間；
  - 移動與縮放：手繪只移動時每個點平移同樣的距離，縮放時點散布在新範圍（減去不變的邊），線寬不變；印章依固有比例縮放並置中；方框等其他種類、不存在的註解與頁碼拒絕；
  - 負向對照：加上作者、不切斷指向、或存檔時重新編號，對應的測試會失敗。
- 主行程（`src-tauri/src/documents.rs`）：新增、改變、移除、復原後編號不變；不存在的註解、頁碼與不是文字的附註被拒絕；作者不允許註解的文件（RC4 語料，`/P -44`）拒絕註解的編輯（含手繪、印章與移動）。手繪與印章的新增、移動、復原、移除與被拒絕的情形（不存在的註解、太小的範圍、頁碼不存在、沒有筆畫）。
- 合約（`crates/ipc_contract`）：註解編輯與 worker 回傳的註解的驗證；附註文字的清理；手繪的筆數與點數上限（各自與全部）、座標、印章與移動的範圍（太小、倒置、不是數字），以及前端 JSON 的形式。
