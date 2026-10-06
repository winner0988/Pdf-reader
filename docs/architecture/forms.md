# 表單填寫與扁平化（B2-09）

工作卡 [#98](https://github.com/winner0988/Pdf-reader/issues/98)；規格 §6「互動表單填寫」；[ADR 0001](../adr/0001-sandboxed-form-js.md)（表單腳本沙盒：本卡不做）。編輯的共通做法（指令、復原、存檔、崩潰復原）見 [ADR 0013](../adr/0013-editing-and-saving.md)、[page-management.md](page-management.md)、[saving.md](saving.md) 與 [crash-recovery.md](crash-recovery.md)；註解的做法見 [annotations.md](annotations.md)。

這份文件先說明 worker 與主行程的部分；畫面（頁面上的輸入元件、`Tab` 順序、「扁平化表單…」）在下一個 PR。

## 不執行任何腳本

- MuPDF 編譯時就沒有 JavaScript 引擎（`javascript_is_compiled_out` 測試）。填寫欄位時 worker 也不通知欄位的腳本（`ignore_trigger_events`）。
- 欄位有腳本時（`/AA` 的格式化、檢查、計算、按鍵，或按鈕的 `/A`，欄位自己或它上面的任何一層都算），`FormField.hasScript` 為真，畫面據此說明「這個欄位的自動計算不會執行」。
- 送出表單、匯入資料等按鈕不在列出的欄位中（只列文字欄位、核取方塊、選項按鈕、下拉選單與清單方塊），維持封鎖（MVP-11）。
- XFA 表單與簽章欄位不處理；有簽章的文件不能扁平化。

## 列出欄位（`get_page_fields`）

回傳 `FormField[]`：

| 欄位 | 內容 |
|---|---|
| `id` | 欄位的 widget 在文件中的物件編號（`FieldId`）。編輯、存檔後不變（存檔不重新編號，見 [annotations.md](annotations.md)「註解的識別碼」） |
| `kind` | `text`、`checkbox`、`radio`、`combo`、`list` |
| `rect` | 頁面空間（PDF 點，左上角為原點） |
| `label` | 欄位的提示文字（`/TU`），沒有時用欄位名稱 |
| `value` | 文字欄位的文字；下拉選單與清單方塊選的選項值；核取方塊與選項按鈕的狀態（`onValue` 或 `Off`，以外觀狀態 `/AS` 為準，沒有時用欄位的 `/V`） |
| `onValue` | 核取方塊與選項按鈕打開時的狀態名稱（外觀中 `Off` 以外的那個）；選項按鈕群組的每個按鈕各有一個 |
| `options` | 下拉選單與清單方塊的選項（`/Opt`：值與顯示的文字） |
| `readOnly`、`required`、`multiline`、`password`、`editable`、`multiSelect`、`maxLen`、`hasScript` | 欄位的旗標與限制 |

- 不列出：隱藏的欄位（`/F` 的 Hidden、NoView、Invisible）、按鈕與簽章欄位。
- `readOnly` 也包括 app 無法正確處理的欄位：可以複選的清單方塊、值太長被截斷的欄位、沒有「打開」外觀的核取方塊與選項按鈕。
- worker 的輸出一律當成不可信任的資料，主行程再以 `validate` 檢查：
  - 文字（提示、值、選項）清理成只有文字：控制字元（換行除外）與看不見的格式字元去掉；單行欄位的換行變成空白；使用者自己的空白保留；
  - 每頁最多 `LIMITS.maxFieldsPerPage`（5,000）個欄位、每個欄位最多 `LIMITS.maxFieldOptions`（1,000）個選項、值最多 `LIMITS.maxFieldValueBytes`（16 KiB）；
  - 編號不重複、範圍是有限值。

## 編輯指令

都是 `apply_edit` 的 `Edit`：記入編輯歷史（可以復原、重做）與崩潰復原日誌，存檔時才寫入檔案。

| `Edit` | 內容 |
|---|---|
| `setFieldValue { page, field, value }` | 設定欄位的值 |
| `flattenForm` | 把表單變成頁面內容 |

**`setFieldValue`**：值最多 `LIMITS.maxFieldValueBytes`，除了換行不能有控制字元或看不見的格式字元；可以是空的（清除欄位）。worker 再依欄位檢查，不符合時什麼都不改變：

- 欄位必須在那一頁、列在清單中、不是唯讀；
- 文字欄位：單行欄位不能有換行；不能超過 `maxLen` 個字元；
- 下拉選單與清單方塊：必須是其中一個選項，或（可編輯的下拉選單）任何文字，或空的；寫入檔案的是選項原本的值；
- 核取方塊：`onValue` 或 `Off`；選項按鈕：只能是它自己的 `onValue`（打開，同一群組的其他按鈕關掉）。

**核取方塊與選項按鈕的狀態**：

- MuPDF 的 `set_value` 只改這個 widget，且把 `/V` 寫成文字；選項按鈕因此不會關掉同群組的其他按鈕。所以 worker 自己做：從這個 widget 往上找到有名稱（`/T`）的欄位，把底下每個 widget 的 `/AS` 設為該狀態（外觀中沒有這個狀態的設為 `Off`），再把欄位的 `/V` 寫成**名稱**（其他閱讀器期待的型別）。
- 群組的深度與 widget 數有上限（循環或過大的表單回 `TooComplex`）。

**`flattenForm`**：MuPDF 的 `bake` 把每個欄位的外觀併入它的頁面，欄位連同 `/AcroForm` 一起移除；之後完整重寫的存檔不再包含欄位，頁面上仍看得到填好的值，文字也找得到（有測試）。

- 有簽章的文件拒絕（簽章會失效）。
- 這是一個可以復原的編輯；「另存新檔」由畫面接著提供。

## 權限（MVP-19）

`DocumentPermissions.fillForms`：`/P` 的第 9 位元（修訂版 3 以上）或第 6 位元（註解，它涵蓋填寫表單）有一個設定就允許；修訂版 2 沒有第 9 位元（它是保留位，一律是 1），只看第 6 位元。沒有權限時主行程拒絕 `setFieldValue` 與 `flattenForm`，欄位仍然列出。以擁有者密碼開啟時全部允許（#88）。見 [encryption.md](encryption.md)「權限」。

## 測試

- worker（`crates/pdf_worker/src/engine.rs`，語料 `benign/form-fields.pdf`，由 `tests/corpus/generate.py` 產生）：
  - 列出欄位：種類、位置、值、選項、旗標、提示文字與名稱、群組的每個按鈕；
  - 填寫文字（含中文與多行）、核取方塊、選項按鈕（整組一起變）、下拉選單、清單方塊，存檔、重新開啟後值都在；按鈕的值是名稱；核取方塊可以關掉，欄位可以清空；
  - 不能有的值一律拒絕、什麼都不改變：唯讀欄位、太長、單行欄位有換行、不是選項、核取方塊的其他值、選項按鈕關掉、不在那一頁或不存在的欄位；
  - 腳本：`malicious/field-aa.pdf` 的欄位 `hasScript` 為真，填寫後值改了，腳本沒有執行；沒有腳本的樣本都是假；
  - 扁平化：沒有欄位、檔案中沒有 `AcroForm`、`/FT`、`/Widget`，填的值在頁面文字中；有簽章的文件拒絕；
  - 權限位元（含修訂版 2 的例外）。
  - 負向對照：選項按鈕只改自己的 widget，測試失敗。
- 主行程（`src-tauri/src/documents.rs`）：填寫、復原、重做（編號不變）、存檔後的值、扁平化與它的復原；不是文字的值、不存在的頁與欄位、唯讀欄位、單行有換行都被拒絕且不算編輯；作者不允許時（RC4 語料）拒絕。
- 合約（`crates/ipc_contract`）：編輯與 worker 回傳的欄位的驗證、欄位文字的清理。
