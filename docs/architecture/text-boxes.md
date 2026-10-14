# 文字方塊（ADR 0018）

[ADR 0018](../adr/0018-text-boxes-and-fonts.md)：文字方塊是頁面上的 FreeText 註解，外觀由 worker 以所選字型的子集自己產生。實作分成五張卡（[#224](https://github.com/winner0988/Pdf-reader/issues/224)–[#228](https://github.com/winner0988/Pdf-reader/issues/228)），第二階段的匯入字型檔是 [#229](https://github.com/winner0988/Pdf-reader/issues/229)。這份文件隨每張卡增補；已經做完的在下面，其餘見「尚未實作」。

## 已安裝字型的清單與讀檔（B2-27，#224）

`src-tauri/src/fonts.rs`；命令 `get_fonts`。

- **來源**：Windows 的登錄檔 `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts`（所有使用者的，值是檔名，在 Windows 的字型資料夾裡）與 `HKCU` 的同一個位置（使用者自己安裝的，值是完整路徑；在 Windows 11 對字型檔按右鍵的「安裝」只給目前使用者，不需要系統管理員）。只讀不寫；用 `windows-sys` 的 `Win32_System_Registry`（只多這一個 feature，沒有新套件），`unsafe` 限於這個模組。
- **名稱**：值的名稱是 Windows 叫它的名字，後面有種類 `(TrueType)`；去掉種類，再當成純文字處理（控制字元與看不見的格式字元去掉、空白收斂、最長 `MAX_FONT_NAME_BYTES`）。結尾的 `Bold`、`Italic`、`Oblique`、`Regular` 是樣式，其餘是家族（`Segoe UI Semibold` 是自己的家族）。粗體與斜體各自是登錄檔的一筆。
- **字型集合**（`.ttc`）：一筆登錄檔，名稱用 ` & ` 連著各個字型，順序就是檔案裡的順序，所以每個名字一筆、序號是它的位置。worker 讀檔之後以字型自己的名稱表確認（B2-28）。
- **不列出的**：不是 `.ttf`、`.ttc`、`.otf` 的檔案（點陣的 `.fon`、Type 1），含資料夾或 `..` 的相對路徑，空的或太長的名稱。`.otf` 先列出來：多半是 CFF 輪廓，第一版選了會被拒絕並說明（ADR 0018）。
- **兩個鍵都有同一個名稱**：使用者自己的優先。排序：家族，再正常、粗體、斜體、粗斜體。最多 `MAX_FONTS`。
- **代號**（`FontId`）：由名稱與位置算出來，每次列出都一樣；畫面只拿到代號、名稱、家族、粗體、斜體與來源（第一版只有「已安裝」），**沒有路徑**。
- **每次都重新讀**：在 Windows 裝完字型，再開字型選單就看得到，不必重新啟動 app。
- **讀檔**（`Fonts::read`，主行程內部，不是命令）：代號對應到最近一次列出的檔案；最多 `MAX_FONT_BYTES`（64 MiB，讀取時就限制，連檔案太大時也不讀）；檔案不見了、讀不了、太大都有各自的錯誤。是不是字型由 worker 說（B2-28）；字型的位元組只交給 worker，主行程不解析。

測試：登錄檔的各種名稱與值（集合、`&` 在單一字型裡、使用者的完整路徑、被丟掉的、控制字元、排序、重複、代號）；這台電腦真正的登錄檔至少有 20 個字型，其中有可以讀的，開頭是 TrueType、集合或 OpenType 的標頭；檔案不見了、太大（稀疏檔）、是資料夾。

## 尚未實作
已做：worker 的字型子集、排版與外觀、契約（B2-28，見 [text-box-engine.md](text-box-engine.md)）；`prepare_text_box`、模型庫、復原與日誌（B2-29 第一部分）。尚未：移動、縮放、改字與樣式、搜尋與選取（B2-29）；畫面與輸入法（B2-30）；端對端與 fuzz（B2-31）；匯入字型檔（B2-32）。
