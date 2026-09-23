---
title: "[QA-03] Fuzzing 基礎：IPC 解碼與 PDF 開檔路徑"
labels: task,mvp,qa,area:security,agent:qa,needs-security-review
---

- **需求 ID**：MVP-R2；ADR 0008（隔離不能取代 fuzzing）
- **負責角色**：QA／安全 agent
- **相依**：MVP-04、QA-01

## 目標
對最容易被惡意輸入攻擊的兩個入口建立 fuzzing，並定期自動執行。

## 範圍
- `cargo-fuzz` 目標：
  1. IPC 訊息解碼（主行程接收 worker 訊息的一側，以及 worker 接收主行程訊息的一側）
  2. 開檔 → 取頁數 → 渲染第一頁 → 取目錄 → 取連結（實際呼叫 MuPDF）
- 種子語料取自 QA-01。
- 排程 workflow（Linux runner，每次限時例如 20 分鐘），發現的崩潰樣本存成 artifact。
- `docs/security/fuzzing.md`：如何在本機執行、如何重現與分類崩潰。

## 不做什麼
- OSS-Fuzz 整合、長時間 fuzz 叢集。

## 可動的模組
- `fuzz/`（新增）
- `.github/workflows/fuzz.yml`（新增）
- `docs/security/fuzzing.md`

## 驗收情境
- 本機可執行 `cargo fuzz run <target>`。
- 排程 workflow 成功執行，並在摘要中輸出執行次數與覆蓋率資訊。
- 故意放入一個會 panic 的輸入，確認 workflow 能偵測並保存樣本（驗證後移除）。

## 必跑測試
- fuzz 目標可編譯（加入一般 CI 的建置檢查）。

## 資安限制
- fuzz workflow 權限最小（`contents: read`）；崩潰樣本 artifact 保留天數設短（例如 7 天）。
- 發現的崩潰依 SECURITY.md 處理，不得附在任何公開位置。
