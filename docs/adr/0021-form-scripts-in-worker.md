# ADR 0021：表單腳本以 MuPDF 內建的 mujs 在文件自己的 worker 中執行，只跑欄位的按鍵、格式、驗證與計算

## 狀態
已接受

（負責人於 2026-10-21 接受選項 A〔[#238 的留言](https://github.com/winner0988/Pdf-reader/issues/238#issuecomment-6303086436)〕。落實 ADR 0001 的例外。工作卡 [#238](https://github.com/winner0988/Pdf-reader/issues/238)〔B3-07〕。實作前先做下面的 POC，**不通過就不實作**。）

## 背景
- ADR 0001（已接受）與 CONTEXT.md「表單腳本沙盒」：表單腳本只在使用者主動與表單互動時執行，只能讀寫這份文件的欄位並觸發重新計算，不得連網、存取檔案或啟動程式；`/OpenAction`、`/AA` 這類開檔即自動觸發的動作一律停用。
- ADR 0003 選 MuPDF 的理由之一就是它內建表單腳本引擎（mujs 與 Acrobat 表單 API 的子集）。現在 `pdf_worker` 編譯時**沒有** `js` 功能，測試 `javascript_is_compiled_out` 守著這一點；主動內容一律封鎖，橫幅只告知。
- 常見的表單計算多半是 Acrobat 介面產生的內建函式（`AFSimple_Calculate`、`AFNumber_Format`、`AFDate_FormatEx` 等），但也有自訂腳本；欄位腳本常會用到文件層級腳本（名稱樹裡的 JavaScript）定義的函式。
- MuPDF 的腳本想對外做事（`app.alert`、`app.launchURL`、`this.print`、`this.mailDoc`、`this.submitForm`、選單指令）時，只是把「文件事件」交給應用程式的回呼；應用程式不處理，就什麼都不會發生。
- worker 在 AppContainer 沙盒中、低完整性、停用 win32k、沒有網路，只拿得到主行程交給它的 handle（ADR 0008）。mujs 是 C 程式碼，過去有記憶體安全的漏洞；它在這個沙盒中執行。
- 語料：`malicious/field-aa.pdf` 正好是欄位的按鍵與格式腳本（內容是 `app.alert`）；`js-document-level`、`openaction-js`、`page-aa`、`submitform`、`importdata` 是不能執行或不能對外的動作。

## 選項

| 選項 | 內容 | 代價 |
|---|---|---|
| **A. mujs 在文件自己的 worker 中，只跑欄位事件（建議）** | 打開 MuPDF 的 `js` 功能；只執行欄位 `/AA` 的 K、F、V、C；不處理任何文件事件 | mujs 這塊 C 程式碼進到 worker（在沙盒中）；要改測試、語料的預期與橫幅 |
| B. 另一個專門跑腳本的沙盒行程 | 腳本與文件分在兩個行程 | MuPDF 的表單 API 直接操作 `pdf_document`，拆開等於重寫整套 API：代價太高 |
| C. 不用 JS 引擎，以 Rust 實作常見的 Acrobat 內建函式 | 辨識 `AFSimple_Calculate`、`AFNumber_Format` 等呼叫並自己計算 | 沒有引擎的攻擊面；但自訂腳本、文件層級函式都不支援，表單會部分失效 |
| D. 不做 | 維持編譯排除，說明限制 | 依賴計算欄位的表單不會自動更新（ADR 0001 的「後果」提到的退路：手動計算按鈕） |

## 決定（選項 A）

### 何時啟用
- 開檔時**不啟用**：不執行文件層級腳本、不執行任何腳本。
- 使用者第一次在**有腳本的欄位**輸入或離開欄位時，詢問一次：「這份表單有自動計算的腳本。要在沙盒中執行嗎？它只能讀寫這份表單的欄位。」選「執行」才對這份文件呼叫 `enable_js`（這時文件層級腳本才執行，屬於使用者觸發）；選「不要」就維持現狀。只對這個分頁有效，不記住。

### 執行什麼
- 只有欄位 `/AA` 的 **K（按鍵）、F（格式）、V（驗證）、C（計算）**，計算順序照 `/AcroForm /CO`。
- 一律不執行：`/OpenAction`、頁面的 `/AA`、欄位與小工具的 E、X、D、U、Fo、Bl、PO、PC、PV、PI、按鈕的 `/A`（含 JavaScript、`SubmitForm`、`ImportData`、`Launch`）、連結的 JavaScript。第一版也不跑按鈕的點擊腳本。
- 所有文件事件（alert、print、launchURL、mailDoc、submitForm、選單指令）都不處理；worker 只回報「表單想要做 X，已封鎖」，接到既有的橫幅。

### 上限
- 每個欄位事件是一個 worker 請求，逾時 2 秒；逾時由主行程照既有的方式結束並重啟 worker，文件重新開啟、腳本關閉，橫幅說明「表單的腳本執行太久，已停止」。
- 記憶體由 worker 既有的上限管住。文件裡的腳本合計超過 1 MB：不詢問、不執行，並說明原因。

### 值怎麼記下來
- 編輯日誌（ADR 0013）記錄的是**欄位的結果值**：使用者輸入的值，以及腳本算出來而改變的其他欄位的值，都是一般的表單填寫指令（B2-09）。
- 復原、重做與崩潰復原只重播值，**不再執行腳本**：結果確定，也不會因為重播而再跑一次不受信任的程式碼。
- 存檔照表單填寫的規則；外觀由 MuPDF 重新產生。

### 測試與語料
- `javascript_is_compiled_out` 改成：開檔後沒有任何腳本執行；沒有使用者動作前 `enable_js` 沒被呼叫；只有 K、F、V、C 會執行。
- 語料 `manifest.json` 的預期跟著改：`field-aa` 的腳本只在輸入時執行，而且 `app.alert` 沒有任何效果；其他惡意樣本完全不變。
- 新增一個由腳本產生的計算表單（兩個數字欄位、一個 `AFSimple_Calculate("SUM")` 的總計、`AFNumber_Format` 的格式）。
- 新增 fuzz 目標：對 fuzz 出來的表單送欄位事件。

### POC（不通過就不實作）
1. 以 `js` 功能建置 worker，記下大小的變化；`cargo deny`、Fuzz 的建置照常通過。
2. 語料的 `openaction-js`、`page-aa`、`js-document-level`、`submitform`、`importdata` 開啟與操作後沒有任何效果；`field-aa` 只在輸入時執行，`app.alert` 沒有效果。
3. 計算表單：輸入後總計更新、格式正確，存檔後在其他閱讀器看得到值。
4. 計算腳本是 `while(true){}`：2 秒後 worker 重啟、介面沒有卡住、文件重新開啟且腳本關閉。
5. 文件層級腳本在開檔時改一個欄位的值：使用者同意前，值沒有改變。
6. 欄位事件的 fuzz 跑 30 分鐘沒有崩潰（崩潰也只結束 worker）。

## 後果
- 換來：依賴計算欄位的表單可以用，而且仍在 worker 的沙盒中；腳本對外的路全部不通（沒有回呼、沒有網路、沒有檔案）。
- 付出：
  - mujs 這塊 C 程式碼成為新的攻擊面；被攻破的影響停在 worker 的沙盒（與 MuPDF 解析 PDF 相同的層級）；
  - 行為以 MuPDF 實作的 Acrobat API 子集為準，與 Acrobat 不同的地方要逐一記錄；
  - 測試、語料預期、橫幅文字與使用手冊要一起改。
- 與其他 ADR 的關係：落實 ADR 0001；ADR 0003（MuPDF）、0008（worker 權限不變）、0013（日誌記錄值，不記錄腳本）。
- 之後若要改變：POC 不通過時改走選項 C（只支援常見的內建函式）；要讓按鈕的點擊腳本執行，另寫 ADR。
