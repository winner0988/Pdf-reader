# PDF Reader 使用手冊

PDF Reader 是在 Windows 上閱讀與編輯 PDF 的 app，所有處理都在你的電腦上完成：不會自行連網，也不收集任何資料。

這份手冊依「想做什麼」排列。畫面上的文字以本手冊的「」標示，例如「⋯」選單中的「另存新檔…」。

## 目錄

1. [安裝與開始使用](01-install-and-start.md)：安裝、設為預設的 PDF 閱讀器、開啟檔案、分頁、最近開啟的檔案、有密碼的文件
2. [閱讀](02-reading.md)：縮放與旋轉、側欄（目錄、縮圖、附件與遮蔽）、搜尋與在所有文件中搜尋、選取與複製、掃描頁的文字辨識、列印、深色模式
3. [編輯與儲存](03-editing.md)：整理頁面、編輯書籤、插入其他檔案的頁面、螢光筆與附註、畫筆與印章、文字方塊、表單、復原、儲存、當機後還原
4. [匯出與轉檔](04-export-and-convert.md)：匯出文字、Word、圖片與拆分 PDF；匯出註解摘要；隱私匯出；分享前淨化；頁碼與 Bates 編號；加密副本；壓縮圖片；從檔案建立 PDF；批次處理
5. [安全與隱私](05-security-and-privacy.md)：不連網的承諾、已封鎖的內容、外部連結、數位簽章與簽署、文件權限、機敏文件、安全遮蔽、這台電腦上保存的資料
6. [快捷鍵與無障礙](06-shortcuts-and-accessibility.md)
7. [常見問題與已知限制](07-faq.md)

## 系統需求

- Windows 11（64 位元）。
- 畫面使用 Windows 11 內建的 Microsoft Edge WebView2。安裝檔不會下載任何東西；WebView2 被移除的電腦（或 Windows 10）要先自行安裝 WebView2 Runtime。
- 安裝時需要系統管理員權限（程式裝在 `C:\Program Files\PDF Reader\`）。

## 五分鐘上手

1. 執行安裝檔（見[安裝](01-install-and-start.md#安裝)）。
2. 開啟 PDF Reader，按「選擇檔案…（Ctrl+O）」，或把 PDF 拖到視窗裡。
3. 用工具列的「放大」「縮小」「符合寬度」調整大小，`Ctrl+F` 搜尋文字。
4. 要畫重點：選取文字，按右鍵選「螢光筆」。
5. `Ctrl+S` 儲存。

## 這個 app 不會做的事

- **不會自行連網。** 唯一的例外是你在「設定」按下「檢查更新」時，向 GitHub 查詢一次最新的版本號碼。
- **不收集資料，不回報錯誤。**
- **不執行 PDF 裡的程式。** JavaScript、開檔自動動作、啟動外部程式等一律封鎖，也沒有「允許」的選項（見[已封鎖的內容](05-security-and-privacy.md#已封鎖的內容)）。

## 給開發者

這份手冊寫給使用者。畫面與行為的完整規格在 [docs/ux/screen-map.md](../ux/screen-map.md)，各功能的做法在 [docs/architecture/](../architecture/)。修改畫面文字（`src/i18n/zh-TW.ts`）或行為時，請一併更新這份手冊。
