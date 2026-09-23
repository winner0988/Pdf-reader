<!-- PR 標題請用 Conventional Commits，例如：feat(worker): render pages in child process -->

Closes #

## 需求 ID

<!-- 例如：MVP-R3 虛擬滾動（規格 §2） -->

## 做了什麼

<!-- 條列變更內容。只做工作卡範圍內的事；範圍外的問題請另開 Issue。 -->

## 如何驗證

<!-- 對照工作卡的「驗收情境」，逐條說明怎麼驗證、結果如何。UI 變更請附截圖（勿含私人內容）。 -->

## 檢查清單

- [ ] 只修改了工作卡「可動的模組」列出的檔案
- [ ] 已在本機跑過 AGENTS.md「指令」表中的所有檢查
- [ ] 新增或更新了對應的測試；沒有停用或略過任何測試
- [ ] 沒有新增連網能力、遙測、遠端字型或 CDN
- [ ] 沒有把檔案路徑暴露給前端；worker 沒有拿到任意路徑
- [ ] 沒有提交私人 PDF、秘密或憑證
- [ ] 若變更規格或決策：已更新相關 ADR／CONTEXT.md／規格書

## 安全審查

- [ ] 本 PR 屬於 AGENTS.md「需要人工安全審查的變更」→ 已加上 `needs-security-review` 標籤，並說明風險：

## AI 協作揭露

- 實作 agent：<!-- 例如 Codex -->
- Review agent：<!-- 必須與實作 agent 不同 -->
