---
title: "[B2-10] ADR 0015：OCR（含 POC）"
labels: task,batch-2,area:worker,area:security,agent:core,needs-security-review
---

- **需求 ID**：規格 §2「OCR 引擎」；README：OCR 要先做 POC 並寫 ADR
- **負責角色**：核心 agent
- **相依**：無

## 目標
掃描件（沒有文字層）目前不能搜尋也不能選取。定案 OCR 的整合方式，並以 POC 證明可以在離線、沙盒中辨識語料的掃描頁。

## 範圍
ADR 0015（提議中）比較並建議：

1. **引擎**：
   - Tesseract（規格指定；Apache-2.0；C++，需要建置或隨附執行檔；語言包每種約 10–50 MB）；
   - Windows 內建 OCR（`Windows.Media.Ocr`：離線、使用 Windows 已安裝的語言；不用隨附模型，但綁定 Windows）。
2. **在哪裡執行**：頁面影像來自不受信任的 PDF，OCR 必須在沙盒中（在 `pdf_worker` 內，或另一個同樣受限的行程）；逾時、記憶體與 CPU 上限。
3. **語言包**：規格要求「由使用者自行選擇安裝」，但 app 不連網。
   - 選項：安裝檔隨附常用語言、使用者自行下載後匯入、使用 Windows 的語言。
   - 匯入的檔案要驗證（大小、格式、雜湊值）。
4. **何時執行**：規格寫「開啟掃描件時自動執行」。在背景逐頁辨識、可以取消，並顯示進度；使用中的頁面優先。
5. **結果放哪裡**：只在記憶體中供搜尋（MVP-10）與選取（MVP-15），或寫入 PDF 的隱藏文字層（需要 B2-02 的存檔）。
6. **授權與體積**：對安裝檔大小的影響；第三方授權清單。

POC：在測試中辨識 `benign/image-only.pdf`（或新增的掃描樣本）的頁面，得到預期的英文與中文字（中文另需語言資料，POC 可以只做英文，ADR 說明中文的做法）。

## 不做什麼
- 不做 UI、設定頁與搜尋整合（ADR 接受後另開卡）。
- 不連網下載任何模型。

## 可動的模組
- `docs/adr/0015-*.md`、`docs/adr/README.md`
- POC：`crates/pdf_worker/tests/` 或獨立的 POC crate；`tests/corpus/`（必要時新增掃描樣本）

## 驗收情境
- ADR 0015 為「提議中」，每個決定點都有選項、建議與理由，包含對安裝檔大小與授權的影響。
- POC 在 CI 上辨識出預期的文字，而且在沙盒限制下執行（AppContainer、無網路）。

## 必跑測試
- AGENTS.md 指令表；POC 測試。

## 資安限制
- 新增或建置 C／C++ 函式庫、隨附執行檔都需要 `needs-security-review`，並說明來源。
- OCR 不得連網，不得寫入任何檔案；語言資料只從使用者選的檔案或安裝目錄讀取。
