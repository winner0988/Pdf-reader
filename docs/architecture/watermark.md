# 浮水印（B3-04）

工作卡 [#235](https://github.com/winner0988/Pdf-reader/issues/235)（批次處理的四項操作之一）；規格 §7「批次處理佇列」。批次處理見 [batch.md](batch.md)。

這一部分是做浮水印副本的能力（含批次處理介面與 worker 核心）：契約、畫面選項、worker 的請求與測試，已接上批次處理佇列。

## 浮水印是什麼

| 項目 | 規則 |
|---|---|
| 內容（`WatermarkContent`） | 一行字（`text`，最多 100 個字元，不能換行或有控制字元；`size` 8～200 點；`color`），或一張圖片（`picture`，寬 16～1200 點，比例不變） |
| 透明度（`opacity`） | 5～100%；以 ExtGState 的 `/ca`、`/CA` 畫，底下的頁面看得見 |
| 方向（`diagonal`） | 水平，或沿著頁面的對角線（從左下往右上，依頁面「顯示時」的形狀） |
| 位置（`place`） | 中間、上方或下方（離邊 36 點），依頁面顯示時的方向：轉過的頁面也是正的 |
| 頁面（`pages`） | 全部、只有第一頁、第一頁以外。批次處理裡每個檔案的頁數不同，所以不用頁碼 |
| 大小 | 太大時縮小到頁面的 90%（水平時看寬，對角線時看對角線長），不會放大 |

## 做法

- worker 請求 `WatermarkCopy { doc, file, watermark, picture }`，與隱私匯出相同：開著的文件**先存到記憶體再重開**，浮水印畫在第二份上，再寫進只能寫入的 handle；開著的文件不變。回 `Saved`。
- **畫成頁面內容，不是註解**：每一頁的 `/Contents` 變成 `[q, 原本的內容…, Q 與浮水印]`，所以原本的內容留下的繪圖狀態（顏色、座標轉換）不會影響浮水印。浮水印本身是一個 Form XObject（`/PdfReaderWatermark`，放進頁面的 `/Resources`；頁面的資源是繼承來的時候，先複製一份給它自己）。每個閱讀器都會顯示，也不能點一下就刪掉；複製文字時找得到浮水印的字。
- **字**：與文字方塊（ADR 0018）相同的方式，用字型的**子集**排成一行，附 ToUnicode，所以其他閱讀器顯示得出來、搜尋得到。字型是 worker 本來就內建、給中文用的 Droid Sans Fallback（也有拉丁字母）：**不讀這台電腦的任何字型**。
  - `mupdf-fonts-droid` 原本就因為 `mupdf` 的 `bundled-fonts-droid` 編進 worker，這裡只是直接取用它的位元組，沒有新增程式碼。
  - 需要排版的文字（阿拉伯文、希伯來文、印度文字等）與文字方塊一樣被拒絕。
- **圖片**：與自訂印章（B2-08）相同，是主行程交來的 PNG，只有像素（`stamp_png_size` 先檢查，最多 1 MB）；放進 PDF 時重新編碼。
- **線上的形式**：前端用的 `Watermark` 的內容是給 JSON 用的帶標籤列舉，postcard 帶不了（與 `Edit`／`WorkerEdit` 相同），所以送給 worker 的是同樣欄位的 `WorkerWatermark`。
- **拒絕**：
  - 加密的文件：worker 不留密碼，副本無法再加密，與隱私匯出相同。
  - 有數位簽章的文件可以做，但副本的頁面已經不是簽署的那些，簽章在副本中不成立。批次處理的畫面會說明。

## 測試

- `ipc_contract`：規則（字數、換行與看不見的字元、字級、透明度、圖片寬度、多的欄位被拒絕）；`WatermarkCopy` 經 postcard 往返不變，而前端的形式確實無法經 postcard 傳送。
- worker（單元，`engine/watermark.rs`）：
  - 字在每一頁上、在中間、水平，文件本身沒有，副本沒有註解；
  - 只有第一頁、第一頁以外；
  - 對角線往右上；上方與下方；
  - 轉過 90° 的頁面上字是正的；
  - 透明度 30% 時底下的白色透出來，100% 時不會；
  - 圖片保持比例、在中間；
  - 不能做的被拒絕且什麼都沒寫；加密的文件被拒絕。
- worker（真的沙盒，`tests/watermark.rs`）：
  - 副本第一頁的文字有浮水印的字，文件本身沒有；
  - 圖片浮水印用印章圖片（`prepare_stamp_image`）的 PNG；
  - 不是 PNG、字卻帶圖片、圖片卻沒有：被拒絕，worker 繼續服務；
  - 加密的文件被拒絕。
- fuzz：`worker_messages` 的種子有兩種浮水印請求，`edit_document` 有 `Edit::AddWatermark` 的隨機生成測試。

## 加在開著的文件（B6-06，#591）

為目前開啟的文件加上文字或圖片浮水印，直接修改文件內容，並支援復原與重做。

### 編輯形式與 IPC

- 前端送出 `Edit::AddWatermark { pages, watermark, image }`，主行程檢查後轉成 `WorkerEdit::AddWatermark { pages, watermark, png }`。
- **目標頁面**：`pages` 陣列直接包含欲套用浮水印的頁碼清單（0-based）。`watermark.pages` 固定設定為 `"all"`，因為具體頁面過濾已由 `pages` 欄位負責。
- **圖片印章路徑**：圖片浮水印使用既有的 `pick_stamp_image` 與 `prepare_stamp_image` 流程。主行程權限檢查放寬為允許註解或允許修改（`document.info.permissions.annotate || document.info.permissions.modify`）。圖片由主行程保管於記憶體中（`pictures.rs`），並在存檔與復原日誌（`recovery.rs`）中維護關聯，防止未存檔前丟失。

### 限制與權限

- **已簽署的文件**：禁止加浮水印（回報 `signed` 錯誤）。因為浮水印直接改動頁面內容串流，會破壞既有的數位簽章。
- **加密的文件**：開啟時已解密，只要文件的權限允許修改（`/P` 第 4 位元），即可加浮水印並在存檔時重新加密。

### 多次加浮水印與資源名稱配置

- 在批次處理中原先固定使用 `/Resources /XObject` 名稱 `/PdfReaderWatermark`。若在同一頁上加第二次浮水印，會覆蓋既有的 Form XObject 定義。
- 為了支援多次加浮水印以及既有浮水印文件的處理，worker 在加入浮水印時透過 `unused_watermark_name` 動態搜尋頁面的 `/Resources /XObject`：
  - 依序嘗試 `PdfReaderWatermark`、`PdfReaderWatermark2`、`PdfReaderWatermark3` … 直到 `PdfReaderWatermark100`。
  - 當同一頁浮水印數量超過 100 時拒絕。
  - 此機制同時修復了批次處理對已有浮水印之檔案處理時舊浮水印被覆蓋的問題。

### 預覽與近似

- 浮水印對話框提供即時 HTML5 Canvas 2D 畫布預覽，模擬旋轉、縮放、透明度與位置配置。
- 機敏文件（`sensitive === true`）不呼叫渲染器以空白背景預覽。
- 預覽中採用的系統字型與 worker 內建的 Droid Sans Fallback 可能略有差異，對話框有提示文字說明。
