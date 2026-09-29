---
title: "[B2-05] 頁面管理：旋轉、刪除、排序、插入空白頁"
labels: task,batch-2,area:worker,area:ipc,area:ui,agent:core,agent:frontend,needs-security-review
---

- **需求 ID**：規格 §6「支援插入、刪除、旋轉頁面順序」
- **負責角色**：前端 agent（縮圖上的操作）＋核心 agent（編輯指令）
- **相依**：B2-02

## 目標
在縮圖側欄中調整頁面：永久旋轉、刪除、拖曳排序、插入空白頁，存檔後生效，可以復原。

## 範圍
- 縮圖多選（`Ctrl`／`Shift`＋點擊）、拖曳排序、右鍵選單：向左／向右旋轉、刪除、在前面／後面插入空白頁（與相鄰頁同尺寸）。
- 鍵盤：`Delete` 刪除選取的頁面、`Ctrl+Z`／`Ctrl+Y` 復原／重做；拖曳以外的排序方式（例如「移到…」輸入頁碼），讓只用鍵盤也能操作。
- 依 ADR 0013 的編輯指令：`RotatePages`、`DeletePages`、`MovePages`、`InsertBlankPage`，在 worker 中套用後重新回報頁數與頁面尺寸。
- 不能刪除所有頁面（至少留一頁）。
- 目錄（書籤）、連結指向被刪除的頁面時：依 MuPDF 的處理，並在文件中說明。
- 權限（MVP-19）：`/P` 的第 11 位元（組合文件）與第 4 位元（修改）都沒有設定時停用，比照 Acrobat。

## 不做什麼
- 從其他檔案插入頁面、合併與拆分（B2-06）。
- 裁切頁面。

## 可動的模組
- `src/features/thumbnails/`、`src/features/shell/`、`src/i18n/zh-TW.ts`
- `crates/ipc_contract/`、`crates/pdf_worker/`、`src-tauri/src/`

## 驗收情境
- 假設刪除第 3 頁、把第 5 頁移到最前面、第 2 頁向右旋轉，當另存新檔並重新開啟，則頁序與旋轉都正確，頁數少 1。
- 假設做了上述三個動作，當按三次 `Ctrl+Z`，則回到原狀，分頁不再標示未儲存。
- 假設選取了全部頁面，則「刪除」停用。

## 必跑測試
- worker 測試：每個指令、指令的組合、邊界（第一頁、最後一頁、超出範圍的頁碼被拒絕）。
- 前端：多選、拖曳、鍵盤操作。
- E2E：上面第一個情境。

## 資安限制
- 頁碼與指令數都要驗證上限（`ipc_contract` 的 `validate`）。
- 編輯只在 worker 中進行。
