# 文字方塊的 worker 引擎（B2-28，#225）

[ADR 0018](../adr/0018-text-boxes-and-fonts.md) 的實作之一：worker 在頁面上加一個文字方塊，也就是 `FreeText` 註解，外觀（字型子集、排版）由 worker 自己寫。這份文件只講引擎；已安裝字型的清單與讀檔在 [text-boxes.md](text-boxes.md)（B2-27）。

程式在 `crates/pdf_worker/src/engine/text_box.rs`，入口是 `PdfDocument::add_text_box(index, rect, &TextBoxSpec, &TextBoxFont)`。契約型別 `Rgb`、`TextAlign`、`FontRefusal` 與限制常數在 `ipc_contract`。**這張卡沒有 IPC 命令**：`Edit::AddTextBox` 與主行程讀字型位元組接在下一個 PR。

## 輸入

| 項目 | 規則 |
|---|---|
| 文字 | 不得全是空白；最多 `MAX_TEXT_BOX_BYTES`（4096）位元組；`\n` 換行 |
| 大小 | 有限值，`MIN_TEXT_BOX_SIZE`（4）到 `MAX_TEXT_BOX_SIZE`（288）點 |
| 方框 | 頁面座標（左上為原點），有限值，每邊至少 8 點 |
| 字型 | 字型檔的位元組（**不受信任**）、集合內的序號（`.ttc`）、Windows 叫它的名字 |

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

`fsType` 的 0x0004（預覽與列印）可以用，結果的 `view_only` 為真，由上層告訴使用者。

## 文字：只排「一個字元一個字形」的文字

第一版用字型的字元對應表（`cmap`）直接找字形，沒有字形變化。對 Latin、希臘、西里爾與中日韓的字是對的；阿拉伯文與希伯來文、印度與東南亞的文字、疊在字上的符號（組合用變音符號、假名濁點的組合記號）、韓文字母（jamo）、零寬字元與改變文字方向的控制字元（`U+202A`–`202E`、`2066`–`2069` 等）會畫錯或不見，所以**拒絕**（`InvalidEdit`「a script that a text box cannot set」），不默默畫錯。ADR 0018 把這些排在範圍外。已組合好的字（`é`、韓文音節）沒問題。

## 排版

- **斷行**：最後一個放得下的空白處（斷行的空白不留）；沒有空白（中文）就在放不下的字形前。一行至少一個字形，再寬也放。行尾的空白放不下時留在行尾（不影響對齊，畫之前去掉）。
- **標點**：行首不是句號、逗號、右括號等；行尾不是左括號、左引號等，必要時把前一個字形一起帶到下一行（禁則）。
- **行距**：`(ascent - descent + leading) / em`，限制在 1.15 到 1.6 倍字大小之間。
- **對齊**：左、中、右；左右各留 2 點。
- **放不下**：超出方框高度的行畫在方框外，被外觀的 `BBox` 裁掉，結果的 `overflow` 為真，由上層提醒使用者。

## 外觀與註解

註解字典與外觀是 worker 手寫的，每個方塊一份自己的字型子集：

- 子集：只含用到的字形（加 `.notdef`），名稱 `ABCDEF+<PostScript 名稱>`，六個大寫字母由子集的位元組算出，所以同一份子集永遠同名。
- 字型物件：`Type0`／`CIDFontType2`、`Identity-H`、`CIDToGIDMap /Identity`，`/W` 是每個字形的寬度，附 `ToUnicode`，別的閱讀器才搜得到、複製得出來。
- 外觀：一個 Form XObject，`/BBox [0 0 寬 高]`，`/Matrix` 只含線性的部分（頁面的旋轉與上下翻轉，平移由註解的 `/Rect` 決定）；內容是 `BT … Tf rg … Tm <字形> Tj … ET`。轉了 90、180、270 度的頁面上，字仍然是正的。
- 註解：`/Subtype /FreeText`、`/Rect`、`/Contents`（UTF-16 的原文）、`/AP << /N >>`、`/DA`、`/Q`、`/F 4`、`/IT /FreeTextTypeWriter`、`/BS << /W 0 >>`，沒有 `/CL`；私有的 `/PdfReaderTextBox << /Font (名稱) /Face 序號 /Size 大小 /Color [r g b] >>` 記著之後（B2-29）要重畫所需的資訊。
- 輸入、字型、文字與頁面都在放進任何物件**之前**檢查：被拒絕的方塊不在文件裡留下東西（測試比對存檔的位元組）。

## 規則：對文字方塊絕不呼叫 MuPDF 的註解 setter 或 `update()`

`set_contents`、`set_rect` 與 `update()` 會讓 MuPDF 依 `/DA` 重畫外觀（Helvetica 或替代字型），使用者選的字型子集就不在外觀裡了。要改文字、大小或顏色，是重新建一整份外觀；要移動，只改 `/Rect`（外觀的 `/Matrix` 沒有平移，所以不必動它）。

兩個測試守這條規則：`mupdf_draws_the_appearance_again_when_it_is_asked_to_change_the_annotation` 證明 MuPDF 現在確實會換掉外觀的字型（升級 MuPDF 後行為若變，它失敗，規則就可以重新看），`moving_a_text_box_by_its_rect_alone_moves_it_and_nothing_else_changes` 證明只改 `/Rect` 就夠。

## 測試

`crates/pdf_worker/tests/text_box.rs`（11 個）：子集與存檔後重開、顏色與大小、三種對齊、移動、轉過的頁面、守規則、現有的註解機制（選取與移除）看得到它而且移除時字型一起走、各種被拒絕的字型與文字（用改過標籤的真字型）、僅供檢視的字型、放不下、不合理的輸入且文件不變。引擎內的單元測試（9 個）測斷行（空白、長字、中文、標點）、需要字形變化的文字、`UTF-16` 與子集標籤。

## 尚未實作

契約的 `Edit::AddTextBox`、`AnnotationKind::TextBox`、主行程讀字型位元組（接 B2-27 的 `Fonts::read`）（下一個 PR）；移動、縮放、改字、復原、搜尋與選取（B2-29）；畫面與輸入法（B2-30）；端對端與字型的 fuzz（B2-31）；匯入字型檔（B2-32）。
