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

## Windows 內建字型的量測（B2-31，#228）

在開發用的電腦（Windows 11，2026-10-14）對字型資料夾 `C:\Windows\Fonts` 的每一個 `.ttf`、`.ttc`、`.otf` 的每個字型，用 release 建置的 `prepare_text_box` 各做一次（一次性的量測程式，沒有提交）。五段文字：英文 `Hello, World 123`、繁體 `繁體中文測試`、簡體 `简体中文测试`、日文 `日本語のテスト`、韓文 `한국어 테스트`。每台電腦的字型不同，這些數字是「Windows 11 內建加上 Office」的樣子，不是保證。

| 項目 | 量到的 |
|---|---|
| 檔案與字型 | 340 個檔案、366 個字型（集合裡每個字型各算一個）；另有一個 4 KB 的 `mstmc.ttf` 殘檔讀不出來，略過 |
| 檔案大小 | 中位數 0.16 MB；最大的 `mingliub.ttc` 37.1 MB、`mingliu.ttc` 27.7 MB、`simsunb.ttf` 21.9 MB、`msjh.ttc` 21.4 MB、`msyh.ttc` 19.7 MB。都在 `MAX_FONT_BYTES`（64 MiB）以內 |
| 輪廓 | 每一個都有 `glyf`（TrueType 輪廓），沒有 `.otf`、沒有 CFF。所以「CFF 被拒絕」在 Windows 自己的字型上看不到，要使用者自己裝的 Adobe 或 Google 字型才會遇到（以單元測試與 E2E 的改裝字型涵蓋） |
| 授權旗標（OS/2 `fsType`） | 336 個是 `0x0008`（可編輯嵌入）、29 個是 `0`（可安裝嵌入）、1 個是 `0x000e`（Engravers MT，受限授權，被拒絕：`fontNotEmbeddable`）。沒有只准檢視與列印（`0x0004`）的字型，所以那條路徑用改裝的字型測 |
| 可變字型（`fvar`） | 8 個：Bahnschrift、Noto Sans HK／TC、Noto Serif HK／TC、Segoe UI Variable、Sitka Text 與它的斜體；被拒絕：`fontVariable` |
| 彩色字型 | 1 個：Segoe UI Emoji（`COLR`）；被拒絕：`fontColour` |
| 英文那段 | 351 個做得出模型；其餘 15 個是上面的 8 個可變、1 個彩色、1 個受限，以及 5 個沒有英文字母的符號字型（MT Extra、MS Outlook、Segoe Fluent Icons、Marlett、Segoe MDL2 Assets：`fontMissingGlyphs`） |
| 繁體那段 | 33 個字型有全部的字 |
| 簡體那段 | 19 個 |
| 日文那段 | 33 個 |
| 韓文那段 | 3 個（Malgun Gothic 的正常、粗體、半細） |
| 子集的大小 | 英文那段：最小 2.4 KB、中位數 8.5 KB、最大 40.9 KB；繁體那段：中位數 11.0 KB、最大 45.7 KB。也就是 20 MB 的字型做出的模型約 10 KB |
| 時間 | `prepare_text_box` 對每個字型都不到 0.2 毫秒（字型已在記憶體裡）；花時間的是主行程讀 37 MB 的檔案與交給 worker |

哪些家族涵蓋哪些文字（以上面的段落為準，括號是這台電腦有的字型）：

- **繁體、簡體、日文都有**：Microsoft JhengHei（正黑體，含 UI、粗體、細體）、Microsoft YaHei（微軟雅黑，含 UI、粗體、細體）、SimSun／NSimSun（宋體）、MingLiU／PMingLiU／MingLiU_HKSCS／MingLiU_MSCS（細明體、新細明體）、DFKai-SB（標楷體）。
- **繁體與日文，沒有簡體**：Yu Gothic（游ゴシック）、MS Gothic／MS PGothic／MS UI Gothic。
- **繁體、日文與韓文**：Malgun Gothic。

所以：英文用 Arial（還有 Segoe UI、Verdana 等）；繁體中文與簡體中文用 Microsoft JhengHei 或 Microsoft YaHei；日文用 Yu Gothic、Meiryo 或 MS Gothic。畫面開新方塊時的預設順序（微軟正黑體、微軟雅黑、Yu Gothic、Segoe UI、Arial）因此涵蓋了所有常見的情況，而且正黑體一個字型就能顯示繁體、簡體與日文。E2E 的四種文字各自挑第一個有的家族（`tests/e2e/text-box.spec.ts` 的 `SCRIPTS`），沒有時失敗並列出字型選單有哪些，不是靜靜地少測。

## 測試（B2-31）

- **端對端**（`tests/e2e/text-box.spec.ts`，真正的 app 與真正的 Windows 字型）：
  - 英文、繁體中文、簡體中文、日文各放一個方塊：頁面上看得到字（藍色的像素在方塊裡）、搜尋找得到而且標示在方塊裡、拖曳選取後複製得到原本的字，另存新檔後檔案裡有 `FreeText` 和 `PdfReaderTextBox`、只多了子集的大小（不到 400 KB，字型本身是好幾 MB），重新開啟後方塊、字與搜尋都還在；
  - 以工具列按鈕加 Enter 放在頁面中央、`Ctrl+Enter` 儲存；以方向鍵移動、以控制點縮放、Enter 開啟對話框改字與換字型（對話框從方塊現在的樣子開始）、刪除、復原與重做；另存後重新開啟的方塊和結束時一樣；
  - 沒有存檔就結束（像當機一樣）：日誌裡有的是子集（一個十六進位的模型，不到 600 KB），不是字型；重新開啟後還原，復原與重做都能用，另存後還在；
  - 不能用的字型說明原因而且對話框不關：英文字型沒有中文的字（`fontMissingGlyphs`）、可變字型 Bahnschrift（`fontVariable`）、彩色字型 Segoe UI Emoji（`fontColour`），換成能用的字型就存得起來；這一條走的是右鍵功能表；
  - **只在 CI 上做**（登錄使用者自己的字型，會動到這台電腦的設定，本機預設跳過；要做就設 `E2E_REGISTER_FONT=1`）：Arial 的複本以另一個名稱登錄在 `HKCU`（和使用者自己裝的字型一樣），出現在字型選單；對話框開著時檔案被刪掉（`fontUnavailable`）；文件在沒有這個字型的電腦上開啟（選單顯示「這台電腦找不到」，改大小可以、改字要先選別的字型）；受限授權（`fsType` 改成 `0x0002`）被拒絕、輪廓不是 TrueType（`glyf` 換了標籤）被拒絕、只准檢視與列印（`0x0004`）的做得出方塊而且狀態列有說明。每個測試結束一定（`finally`）把登錄值和暫存檔移除。
- **無障礙**（`tests/e2e/accessibility.spec.ts`）：文字方塊的對話框（新增、顯示錯誤、改現有的）與選取方塊時的小工具列，淺色與深色各以 axe 檢查。
- **fuzz**（`fuzz/fuzz_targets/text_box_font.rs`，見 [fuzzing.md](../security/fuzzing.md)）：字型的位元組經 `skrifa` 讀、`subsetter` 裁、放上頁面、渲染、存檔、重新開啟、搜尋的整條路；種子是 worker 附帶的 Droid Sans Fallback 與 runner 上的 TrueType 字型；每週 20 分鐘，修改相關檔案的 PR 跑 2 分鐘。
- 為了不把字型放進 repo（AGENTS.md 原則 7），測試只用這台電腦與 CI 的 Windows 已有的字型；單元測試與整合測試用 worker 附帶的 Droid Sans Fallback（本來就在 repo 的相依套件裡）。

## 尚未實作
已做：worker 的字型子集、排版與外觀、契約（B2-28，見 [text-box-engine.md](text-box-engine.md)）；`prepare_text_box`、模型庫、復原與日誌（B2-29 第一部分）。移動、縮放、改字與樣式（B2-29 第二部分）。搜尋與選取（B2-29 第三部分）。畫面與輸入法（B2-30）。端對端與 fuzz（B2-31）。尚未：匯入字型檔（B2-32）。
