---
title: "[MVP-13] 離線與零遙測守門：cargo-deny、CSP／capability 檢查、離線驗證流程"
labels: task,mvp,area:ci,area:security,agent:release,needs-security-review
---

- **需求 ID**：MVP-R10；ADR 0009
- **負責角色**：Review／Release agent
- **相依**：MVP-01

## 目標
把「不連網、零遙測」從口頭原則變成 CI 會擋的規則，並建立發布前的手動離線驗證流程。

## 範圍
- `deny.toml`（cargo-deny）：
  - advisories：已知弱點
  - licenses：允許清單（依 DEC-02；在 DEC-02 定案前先列出現況並標示待確認）
  - sources：只允許 crates.io
  - bans：禁止 HTTP client 類 crate（清單依實際依賴樹驗證後訂定，確認不與 Tauri 本身衝突）
- 新增 CI 檢查腳本（放在 `scripts/ci/`，由 `Guardrails` 執行）：
  - CSP 不含任何外部來源
  - capability 不含 http、shell 執行、全域 fs 權限
  - 前端建置產物（`dist/`）的 HTML／CSS 中沒有外部 `src`、`href`、`url(...)` 資源引用
- `docs/security/offline-verification.md`：用 Windows 內建工具（資源監視器、防火牆記錄）或 Process Monitor，驗證開檔、捲動、搜尋、點連結後取消的過程中沒有任何對外連線；每次發布前執行。
- 確認沒有任何崩潰回報或日誌上傳；日誌只寫本機，正式建置不含檔案路徑與文件內容。
- 更新 `docs/workflow.md` 的 CI 檢查表。

## 不做什麼
- 安裝時設定系統防火牆規則、更新檢查（依 DEC-01）。

## 可動的模組
- `deny.toml`（新增）
- `scripts/ci/`、`.github/workflows/`
- `docs/security/`、`docs/workflow.md`

## 驗收情境
- 在暫時分支中加入 `reqwest`、`@sentry/browser` 或外部字型 → CI 失敗（驗證紀錄附在 PR，驗證完移除）。
- 在 capability 加入 http 權限，或在 CSP 加入外部網域 → CI 失敗。
- 依 `offline-verification.md` 手動驗證一次，結果附在 PR。

## 必跑測試
- 新增的檢查腳本在 CI 執行並通過。
- `cargo deny check` 通過。

## 資安限制
- 本卡修改 CI 與安全規則，需要負責人親自核准。
- 檢查規則只能收緊，不能為了讓 CI 通過而放寬；確有需要的例外要寫明理由並另開決策卡。
