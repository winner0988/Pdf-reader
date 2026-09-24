# 架構決策紀錄（ADR）

每一個「之後要改會很痛」的決定都要寫成 ADR。詞彙定義見根目錄的 [CONTEXT.md](../../CONTEXT.md)。

## 索引

| 編號 | 標題 | 狀態 |
|---|---|---|
| [0001](0001-sandboxed-form-js.md) | 允許沙盒化表單 JavaScript，而非完全禁用 | 已接受 |
| [0002](0002-remote-resource-default-block.md) | 遠端資源預設封鎖，並提供手動信任例外 | 已接受 |
| [0003](0003-mupdf-engine.md) | 選定 MuPDF 作為 PDF 處理引擎 | 已接受 |
| [0004](0004-sensitive-file-scope.md) | 系統層級外洩防護只套用在手動標記的文件 | 已接受 |
| [0005](0005-batch-background-execution.md) | 批次處理可在背景／系統列繼續執行 | 已接受 |
| [0006](0006-password-credential-store.md) | 加密檔案密碼交由系統憑證庫記住 | 已接受 |
| [0007](0007-tech-stack-and-platform.md) | 技術棧與首發平台 | 已接受 |
| [0008](0008-pdf-worker-isolation.md) | PDF 引擎隔離在 pdf_worker 子行程，前端不直接接觸引擎 | 已接受 |
| [0009](0009-default-network-policy.md) | 預設網路政策 | 提議中 |
| [0010](0010-document-id-write-safety.md) | 文件識別碼寫入 PDF 的安全限制 | 提議中 |
| [0011](0011-license-and-distribution.md) | 專案以 AGPL-3.0-or-later 授權並公開原始碼 | 提議中 |
| [0012](0012-tabs-and-worker-per-document.md) | 多份文件以分頁呈現，每份文件一個 worker，單一執行個體 | 提議中 |

## 規則

- 檔名：`NNNN-英文短名.md`，編號連續、不重複、不重用。
- 內容用繁體中文，至少包含 `## 狀態`、`## 背景`、`## 決定`、`## 後果` 四段；可從 [template.md](template.md) 複製。
- 狀態只能是：**提議中**、**已接受**、**已取代（由 ADR NNNN 取代）**、**已棄用**。
- 已接受的 ADR 不改內容；要推翻就寫一條新的 ADR，並把舊的狀態改成「已取代」。
- **只有專案負責人可以把 ADR 改成「已接受」。** AI agent 只能新增「提議中」的 ADR。
- 新增 ADR 時同步更新上面的索引；CI（`scripts/ci/check-adr.sh`）會檢查編號與索引是否一致。
