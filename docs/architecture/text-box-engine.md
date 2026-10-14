# 文字方塊的 worker 引擎（B2-28，#225）

[ADR 0018](../adr/0018-text-boxes-and-fonts.md) 的實作之一：worker 在頁面上加一個文字方塊，也就是 `FreeText` 註解，外觀（字型子集、排版）由 worker 自己寫。這份文件只講引擎；已安裝字型的清單與讀檔在 [text-boxes.md](text-boxes.md)（B2-27）。

程式在 `crates/pdf_worker/src/engine/text_box.rs` 與它的 `text_box/`（`font.rs` 讀字型與切子集、`layout.rs` 斷行與排版、`pdf.rs` 寫成與讀回 PDF 物件）。契約型別 `Rgb`、`TextAlign`、`TextBoxStyle`、`TextBoxModelId`、`FontRefusal`、`TextBoxModel` 與限制常數在 `ipc_contract`。訊息、編輯與錯誤碼見下面的「契約」，主行程（命令、模型庫、復原、日誌）見「主行程」。

## 兩個步驟與「模型」

做一個文字方塊分兩步：

1. **`prepare_text_box(font, text) -> TextBoxModel`**：讀字型（不受信任）、檢查授權、為每個字元找字形，再把字型切成只有這些字形的子集。不需要文件。
2. **`PdfDocument::place_text_box(index, rect, &model, &style)`**：把模型放進頁面上的一個方框（大小、顏色、對齊在 `style`）。不需要字型。

**模型（`TextBoxModel`）就是再排一次這些字所需要的一切**：文字、字型的名稱與序號、子集（TrueType，約幾 KB）、每個字元的字形號碼與寬度、度量。原因是 ADR 0018：復原會重新開啟文件，把前面所有的編輯再做一次，崩潰復原日誌只有 4 MiB；字型檔最多 64 MiB，所以重做與日誌**只能放子集，不能放字型**。有了模型：

- 重播（復原、重做、崩潰後還原）不需要原來的字型檔，字型被移除了也不怕；
- 方塊可以移到別頁、別的文件、改大小、改顏色、改對齊，都不必再讀字型；
- 只有**改文字**或**換字型**才需要字型，那時重新 `prepare_text_box`。

`add_text_box(index, rect, &spec, &font)` 是兩步合在一起，測試用。

## 輸入

| 項目 | 規則 |
|---|---|
| 文字 | 不得全是空白；最多 `MAX_TEXT_BOX_BYTES`（4096）位元組；`\n` 換行 |
| 大小 | 有限值，`MIN_TEXT_BOX_SIZE`（4）到 `MAX_TEXT_BOX_SIZE`（288）點 |
| 方框 | 頁面座標（左上為原點），有限值，每邊至少 `MIN_ANNOTATION_SIDE_PT`（8）點 |
| 字型 | 字型檔的位元組（**不受信任**）、集合內的序號（`.ttc`）、Windows 叫它的名字 |
| 模型 | 從磁碟（日誌）或別的程序來的模型一律當成不可信任：`TextBoxModel::validate`（長度、數字範圍、每個字元都有字形、子集名稱與標頭）之後，worker 還把子集**再切一次**（`clean_subset`）才嵌入，所以放進檔案的永遠是 `subsetter` 從 `skrifa` 讀得懂的位元組寫出來的 |

不合的輸入回 `InvalidEdit`，文件不變。

## 字型：只收能放心嵌入的

由 `skrifa` 讀、`subsetter` 切；兩者都是記憶體安全的 Rust。檢查依序如下，第一個不合的決定 `FontRefusal`：

| 條件 | 拒絕 |
|---|---|
| 不是字型、序號超出、`head` 壞了、每 em 小於 16 單位 | `NotAFont` |
| 有 `fvar`（可變字型） | `Variable` |
| 有 `COLR`、`CBDT`、`sbix` 或 `SVG `（彩色） | `Colour` |
| 沒有 `glyf` 與 `loca`（OpenType／CFF 等） | `NotTrueType` |
| `OS/2` 的 `fsType` 有 0x0002（受限）、0x0100（不得子集化）或 0x0200（只能嵌入點陣） | `NotEmbeddable` |
| 文字裡有字型沒有字形的字元 | `MissingGlyphs`（不畫成方框） |

`fsType` 的 0x0004（預覽與列印）可以用，模型的 `view_only` 為真，由上層告訴使用者。子集超過 `MAX_TEXT_BOX_SUBSET_BYTES`（2 MiB）回 `SubsetTooLarge`（`limitExceeded`）。

## 文字：只排「一個字元一個字形」的文字

第一版用字型的字元對應表（`cmap`）直接找字形，沒有字形變化。對 Latin、希臘、西里爾與中日韓的字是對的；阿拉伯文與希伯來文、印度與東南亞的文字、疊在字上的符號（組合用變音符號、假名濁點的組合記號）、韓文字母（jamo）、零寬字元與改變文字方向的控制字元（`U+202A`–`202E`、`2066`–`2069` 等）會畫錯或不見，所以**拒絕**（`EngineError::UnsupportedText`），不默默畫錯。ADR 0018 把這些排在範圍外。已組合好的字（`é`、韓文音節）沒問題。

## 排版

排版只用模型裡的字形寬度（千分之一 em），不需要字型：

- **斷行**：最後一個放得下的空白處（斷行的空白不留）；沒有空白（中文）就在放不下的字形前。一行至少一個字形，再寬也放。行尾的空白放不下時留在行尾（不影響對齊，畫之前去掉）。
- **標點**：行首不是句號、逗號、右括號等；行尾不是左括號、左引號等，必要時把前一個字形一起帶到下一行（禁則）。
- **行距**：`(ascent - descent + leading) / em` 限制在 1.15 到 1.6 倍字大小之間，存在模型的 `step`。
- **對齊**：左、中、右；左右各留 2 點。
- **放不下**：超出方框高度的行畫在方框外，被外觀的 `BBox` 裁掉，`place_text_box` 的結果 `overflow` 為真，由上層提醒使用者。

## 外觀與註解

註解字典與外觀是 worker 手寫的，每個方塊一份自己的字型子集：

- 子集：只含用到的字形（加 `.notdef`），名稱 `ABCDEF+<PostScript 名稱>`，六個大寫字母由子集的位元組算出（在 `prepare_text_box`，之後存在模型裡，所以重播時同名）。
- 字型物件：`Type0`／`CIDFontType2`、`Identity-H`、`CIDToGIDMap /Identity`，`/W` 是每個字形的寬度，附 `ToUnicode`，別的閱讀器才搜得到、複製得出來。
- 外觀：一個 Form XObject，`/BBox [0 0 寬 高]`，`/Matrix` 只含線性的部分（頁面的旋轉與上下翻轉，平移由註解的 `/Rect` 決定）；內容是 `BT … Tf rg … Tm <字形> Tj … ET`。轉了 90、180、270 度的頁面上，字仍然是正的。
- 註解：`/Subtype /FreeText`、`/Rect`、`/Contents`（UTF-16 的原文）、`/AP << /N >>`、`/DA`、`/Q`、`/F 4`、`/IT /FreeTextTypeWriter`、`/BS << /W 0 >>`，沒有 `/CL`。
- **私有的 `/PdfReaderTextBox` 字典**（別的閱讀器忽略它）保存整個模型與樣式，讓方塊可以只憑檔案再排一次：`/V`（版本 1）、`/Text`、`/Font`、`/Face`、`/Base`、`/M`（度量與 `step`、`bbox`）、`/G`（每個字元的 `字形號碼 寬度 碼位`）、`/S`（大小）、`/C`（顏色）、`/A`（對齊）、`/Over`（放不下）、`/View`（字型只能檢視與列印）、`/File`（子集的串流）。`PdfDocument::text_box(index, id)` 讀回並以同樣的規則檢查：檔案裡的東西不可信任。
- 做方塊用到的每個物件都在檢查通過**之後**才放進文件：被拒絕的方塊不在文件裡留下東西（測試比對存檔的位元組）。同一個模型放進同樣的文件，得到同樣的物件編號（`AnnotationId`），所以記著註解編號的編輯在重播時仍然對得上。

## 規則：對文字方塊絕不呼叫 MuPDF 的註解 setter 或 `update()`

`set_contents`、`set_rect` 與 `update()` 會讓 MuPDF 依 `/DA` 重畫外觀（Helvetica 或替代字型），使用者選的字型子集就不在外觀裡了。要改文字、大小或顏色，是重新建一整份外觀；要移動，只改 `/Rect`（外觀的 `/Matrix` 沒有平移，所以不必動它）。

兩個測試守這條規則：`mupdf_draws_the_appearance_again_when_it_is_asked_to_change_the_annotation` 證明 MuPDF 現在確實會換掉外觀的字型（升級 MuPDF 後行為若變，它失敗，規則就可以重新看），`moving_a_text_box_by_its_rect_alone_moves_it_and_nothing_else_changes` 證明只改 `/Rect` 就夠。

## 契約

文字方塊的流程照圖片印章（`pick_stamp_image` → `StampImageId` → `Edit::AddImageStamp`）的樣子：先**準備**，編輯只帶準備好的東西的號碼。

- **`prepare_text_box { doc, text, font }`**（命令，`commands.rs`）→ `PreparedTextBox { model, viewOnly }`：`font` 是 `get_fonts`（B2-27）清單裡的字型代號，`text` 受 `PrepareTextBoxArgs` 驗證。主行程讀字型檔、送給 worker（`WorkerRequest::PrepareTextBox { font, text }` → `WorkerResponse::TextBoxPrepared { model }`），模型留在主行程（像圖片），畫面只拿到號碼 `TextBoxModelId` 與「字型只能檢視與列印」。畫面因此在關掉對話框**之前**就知道字型能不能用，不必等編輯失敗。
- **`Edit::AddTextBox { page, rect, model, style }`**：`model` 是上面的號碼，`style` 是 `TextBoxStyle { size, color, align }`。字型不在 `style` 裡（它在模型裡），粗體與斜體也不是欄位：它們是同一家族的另一個字型（`FontInfo` 有 `family`、`bold`、`italic`），畫面準備文字時換一個字型代號就好；沒有變體時畫面說明，不模擬。驗證比照其他註解的編輯（頁碼、`rect` 正的、每邊至少 `MIN_ANNOTATION_SIDE_PT`、有限值，大小 4 到 288 點）。
- **`WorkerEdit::PlaceTextBox { page, rect, model, style }`**：主行程把號碼換成模型再送給 worker（`WorkerEdit::of` 的 `text_box` 解析器；找不到回 `UnknownFile::TextBox`）。`FontData`（字型位元組）的 `Debug` 只印大小。
- **`AnnotationKind::TextBox`**：`page_annotations` 對 app 的文字方塊回這個種類（有私有記號的 `FreeText`；別的程式做的 `FreeText` 仍是 `other`），`text` 是它的文字；可以選取、移動、縮放、改樣式、改字與刪除。
- **錯誤碼**：`FontRefusal` 與 `UnsupportedText` 經 `WorkerErrorCode::Font(…)`、`WorkerErrorCode::UnsupportedText` 到前端的 `ErrorCode`：

| worker | 前端的 `ErrorCode` |
|---|---|
| `Font(NotAFont)` | `fontInvalid` |
| `Font(NotTrueType)` | `fontNotTrueType` |
| `Font(Variable)` | `fontVariable` |
| `Font(Colour)` | `fontColour` |
| `Font(NotEmbeddable)` | `fontNotEmbeddable` |
| `Font(MissingGlyphs)` | `fontMissingGlyphs` |
| `UnsupportedText` | `textNotSupported` |
| `SubsetTooLarge` | `limitExceeded` |
| （主行程）字型檔不在了或讀不到 | `fontUnavailable` |

訊息文字在 `src/i18n/zh-TW.ts`，集中管理。

- **JSON 與 postcard**：`TextBoxModel` 在 worker 與主行程之間用 postcard（子集是原樣的位元組），在日誌裡用 JSON（子集是小寫十六進位文字，像圖片）；讀日誌時一樣當成不可信任的資料（`validate`）。

## 主行程（B2-29）

- **命令 `prepare_text_box`**（`commands.rs`）：驗證引數，用字型代號向 `Fonts` 要名稱與檔案（清單裡沒有時重新讀一次登錄檔；檔案不在、讀不到或太大回 `fontUnavailable`），交給 `Documents::prepare_text_box`。前端永遠拿不到路徑與字型的位元組。
- **`Documents::prepare_text_box`**：作者不允許註解時拒絕；請文件自己的 worker 準備（`PrepareTextBox`）；worker 回的模型再驗證一次；放進這份文件的**模型庫**（`text_boxes.rs`，比照圖片庫 `pictures.rs`），回傳號碼。
- **模型庫**：每份文件一個，模型以號碼命名；準備新的文字時，沒有任何編輯（做過的與復原後還能重做的）用到的模型會被放掉，所以對話框裡反覆準備不會堆積；號碼不重複使用；最多 1,000 個、子集合計最多 64 MiB，滿了回 `limitExceeded`（請使用者先存檔）。
- **編輯**：`apply_edit(AddTextBox)` 把號碼換成模型再送給 worker（`WorkerEdit::PlaceTextBox`）；找不到號碼回 `invalidArgument`。復原、重做、worker 重開、崩潰後還原都用同一個換法，所以**字型檔只在準備時讀一次**，之後的重播送的是幾 KB 的模型。
- **日誌**：日誌多一個 `textBoxes`（見 [crash-recovery.md](crash-recovery.md)）；寫日誌前先確認放得下（`Journals::check`），放不下的編輯被拒絕並請使用者先存檔；開啟檔案時日誌裡的模型當成不可信任的資料檢查。字型被解除安裝或換了電腦都不影響還原。

## 移動、縮放、改樣式與改字（B2-29）

方塊的私有記號裡有整個模型，所以**只憑檔案就能再排一次**，不需要字型：

- **移動**（`setAnnotationRect`，`move_text_box`）：大小沒變時只改註解的 `/Rect`，其他什麼都不動；大小變了（縮放）就用檔案裡的模型重新排版（斷行會不同）。從來不呼叫 MuPDF 的註解 setter。
- **改樣式**（`setTextBox` 沒有 `model`，`set_text_box`）：大小、顏色、對齊換掉，文字與字型是方塊原來的；方塊留在原處。
- **改字或換字型**（`setTextBox` 帶 `model`）：畫面先用 `prepare_text_box` 把新文字與新字型準備好，編輯帶號碼。新的子集、字型物件與外觀取代舊的，註解**還是同一個**（編號不變，所以記著註解編號的編輯與復原仍然對得上），舊的字型物件不再被引用，完整存檔時消失。
- 這些都是用 `replace_text_box`：只對註解字典逐項 `dict_put`（`/Rect`、`/Contents`、`/DA`、`/Q`、`/AP`、私有記號），不經過 MuPDF 的註解函式。
- **找不到原來的字型時**（解除安裝、換電腦）：方塊照原樣顯示與列印，移動、縮放、改大小、顏色與對齊、刪除都照常（比 ADR 0018 要求的更好：模型在檔案裡）；只有改字或換字型要先選一個字型。
- `get_text_box` 回方塊的文字、字型名稱、樣式、`viewOnly` 與 `overflow`，用來填入對話框。

## 測試

- `crates/pdf_worker/tests/text_box.rs`（16 個）：子集與存檔後重開、英文／繁體中文／簡體中文／日文各一個、顏色與大小、三種對齊、移動、轉過的頁面、守規則、現有的註解機制（選取與移除）看得到它而且移除時字型一起走、只有帶 app 的記號的 `FreeText` 才是文字方塊、各種被拒絕的字型與文字（用改過標籤的真字型）、需要字形變化的文字、僅供檢視的字型、放不下、不合理的輸入且文件不變；**模型**：幾 KB、不用字型就能放進別的文件並改大小、與 `add_text_box` 得到同樣的註解編號、從存好的檔案讀回、被竄改的模型不做出方塊。
- `crates/pdf_worker/tests/text_box_worker.rs`（3 個）：在真正的沙盒 worker 裡，字型以位元組送去準備，模型在**另一個**沒見過字型的 worker 裡放成方塊、列出、存檔、只含子集；被拒絕的字型與文字各有自己的錯誤碼；被竄改的模型不做出方塊也不傷 worker。
- 引擎內的單元測試：斷行（空白、長字、中文、標點）、需要字形變化的文字、`UTF-16` 與子集標籤、度量的單位。
- `ipc_contract`：模型、樣式、編輯與準備文字的引數的驗證與前端的 JSON 形式；模型經 postcard 與 JSON 來回不變；錯誤碼對應。
- `crates/pdf_worker/tests/text_box.rs` 還有（20 個中的一部分）：透過註解編輯移動後顏色與字型都還在、縮放後用檔案裡的模型重排而且註解編號不變、換樣式與換字的文字方塊舊字型隨完整存檔消失、不是 app 的文字方塊與不合法的矩形不會改到文件。沙盒 worker 的測試也走過 `GetTextBox`、移動、`SetTextBox`（不帶字型）與換字。
- 主行程（用這台電腦真正的 Arial）：準備、放進六個方塊、復原、重做、worker 死掉後重開、存檔後重新開啟；被拒絕的字型與文字各有自己的錯誤碼，文件沒有改變；作者不允許註解時先拒絕；日誌裡的文字方塊只有幾 KB，崩潰後下次開啟時還原，之後還能復原；模型庫的編號、放掉與上限；日誌裡的模型當成不可信任的資料（缺了、重複、不合格、不是十六進位、多出來的欄位）。

## 尚未實作

搜尋與選取（B2-29 的其餘部分）；畫面與輸入法（B2-30）；端對端與字型的 fuzz（B2-31）；匯入字型檔（B2-32）。
