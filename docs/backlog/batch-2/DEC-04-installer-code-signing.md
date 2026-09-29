---
title: "[DEC-04] 決定安裝檔與執行檔的程式碼簽章"
labels: decision,batch-2,area:build,agent:owner
---

- **需求 ID**：REL（發行）；README「下一批候選：安裝包簽章」
- **負責角色**：負責人
- **相依**：DEC-03（ADR 0011 決定是否公開發布）

## 目標
目前的安裝檔與 `pdf-reader.exe`、`pdf_worker.exe` 都沒有簽章：Windows SmartScreen 會警告「不明的發行者」，使用者也無法確認檔案沒有被竄改。決定要不要簽章、用什麼憑證、金鑰放在哪裡。

## 範圍
比較並選擇：

| 選項 | 內容 | 代價 |
|---|---|---|
| A. Azure Trusted Signing | 雲端簽章服務，金鑰由 Microsoft 保管，CI 以 OIDC 取得權限 | 每月費用；需要身分驗證；簽章時 CI 要連到 Azure（只在發行的 CI 工作，不在 app 內） |
| B. OV／EV 程式碼簽章憑證 | 向憑證機構購買；2023 年起私鑰必須放在硬體（HSM 或 USB token） | 每年費用；CI 無法直接使用 USB token，可能要在負責人電腦上手動簽章 |
| C. 不簽章 | 在 README 與發行說明中說明 SmartScreen 的警告，並提供 SHA-256 雜湊值 | 使用者體驗與信任度較差 |

- 決定後寫成 ADR（提議中由 agent 草擬，負責人接受），實作另開卡。

## 不做什麼
- 不購買或申請任何服務（負責人決定後自行處理）。
- 不修改 CI。

## 可動的模組
- `docs/adr/`（新的 ADR）、`docs/architecture/packaging.md`

## 驗收情境
- ADR 寫明選擇、金鑰保管方式、哪些檔案要簽（安裝檔、兩個執行檔）、時間戳記伺服器。

## 必跑測試
- CI `Guardrails`（`check-adr.sh`）通過。

## 資安限制
- 私鑰與憑證密碼不得出現在 repo、CI 日誌或 PR 中。
- 簽章只在發行用的 CI 工作進行；app 本身不因此連網。
