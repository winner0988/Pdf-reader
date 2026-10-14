# 獨立審查與驗證（給 Antigravity 等其他 agent）

[AGENTS.md](../../AGENTS.md)：實作的 agent 不能審查自己的 PR。這份說明給負責審查與驗證的另一個 agent。負責人只要貼一句：

> 依 docs/agents/independent-review.md 審查 PR #N

或：

> 依 docs/agents/independent-review.md 除錯：<失敗的 CI 連結、測試名稱或症狀>

## 邊界
- 不合併、不關閉 PR、不推到 `main`、不改 repo 或帳號設定、不改 ADR 的狀態。
- **不在共用的工作目錄動手**（那裡可能有別人未提交的變更）：用自己的 worktree（見下）。
- 不提交、不推送任何檔案，除非任務明說；結果用 PR 留言回報。
- 輸出不放私人 PDF 的內容、密碼、金鑰。

## 審查 PR #N
1. `gh pr view N --json title,body,files,headRefName,labels` 與 `gh pr diff N`。先讀 AGENTS.md 的「不可違反的原則」與「需要人工安全審查的變更」。
2. 自己的 worktree：
   ```
   git fetch origin pull/N/head:review/pr-N
   git worktree add <暫存資料夾>/review-pr-N review/pr-N
   ```
   做完用 `git worktree remove` 與 `git branch -D review/pr-N` 清掉。
3. 只跑和變更有關的指令（AGENTS.md「指令」表）；**CI 已經跑過的不要重跑**，除非要重現問題。
4. 看什麼：有沒有違反七條原則；新依賴、權限、`unsafe` 有沒有說明；文件、程式與測試是否一致；測試真的會失敗嗎（把修正拿掉試一次）；有沒有超出卡片的範圍；程式碼註解與 commit 用英文、介面文字集中在 `src/i18n/zh-TW.ts`。
5. 用 `gh pr comment N --body-file <檔案>` 留言。

## 除錯
給一個失敗的 CI 或 E2E：`gh run view <id> --log-failed` 只看尾端；E2E 的 `tests/e2e/test-results/*/error-context.md` 與截圖。在自己的 worktree 重現，回報**有證據的根因**與最小的修正建議（diff）。要提交修正就開新分支與 PR，標題與說明遵守 AGENTS.md。

## 回報格式
**獨立審查（<agent 名稱>，<日期>）**
- 結論：可合併／需修改／需要負責人的安全審查
- 跑了什麼：指令與結果，一行一個
- 發現：嚴重度、`檔案:行`、一句話、怎麼重現
- 沒檢查的：
