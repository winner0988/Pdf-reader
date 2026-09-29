---
title: "[B2-07] 註解：螢光筆與文字附註"
labels: task,batch-2,area:worker,area:ipc,area:ui,agent:core,agent:frontend,needs-security-review
---

- **需求 ID**：規格 §6「註解標記：螢光筆」
- **負責角色**：前端 agent＋核心 agent
- **相依**：B2-02；MVP-15（文字選取）

## 目標
選取文字後加上螢光筆標示（可選顏色），或在頁面上加文字附註；存成標準的 PDF 註解，其他閱讀器也看得到。

## 範圍
- 選取文字 → 右鍵「螢光筆」或工具列按鈕；四種顏色。沿用 MVP-15 的選取範圍（行的四邊形）產生 `Highlight` 註解的 `QuadPoints`。
- 文字附註（`Text`／`FreeText` 註解）：在頁面上點一下放置，輸入文字。
- 選取既有的註解：刪除、改顏色、編輯附註文字；復原／重做。
- 文件原有的註解照常顯示（MuPDF 已渲染）；可以選取並刪除。
- 註解作者欄位預設空白（不自動填入 Windows 使用者名稱）。
- 權限（MVP-19）：`/P` 的第 6 位元（註解）沒有設定時停用。

## 不做什麼
- 手繪、印章（B2-08）；註解清單側欄；回覆與討論串。

## 可動的模組
- `src/features/text/`、`src/features/viewer/`、`src/features/shell/`、`src/i18n/zh-TW.ts`
- `crates/ipc_contract/`、`crates/pdf_worker/`

## 驗收情境
- 假設選取 `benign/mixed-text-zh-en.pdf` 的「Privacy」並標示螢光筆，當存檔並重新開啟，則該字有 `Highlight` 註解，`QuadPoints` 覆蓋這個字。
- 假設刪除剛加的註解後存檔，則文件中沒有它。
- 新增的註解沒有作者名稱，也沒有電腦名稱等資訊。

## 必跑測試
- worker 測試：註解的類型、顏色、`QuadPoints`、沒有作者欄位。
- 前端：選取後的選單、顏色、刪除。
- E2E：標示並存檔後重新開啟。

## 資安限制
- 附註文字長度有上限，並經過 `ipc_contract` 的驗證。
- 不寫入任何可識別使用者的資訊。
