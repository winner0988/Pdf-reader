---
title: "[B2-01] ADR 0013：編輯與存檔架構（含 POC）"
labels: task,batch-2,area:worker,area:app,area:ipc,agent:core,needs-security-review
---

- **需求 ID**：規格 §4–§6（編輯、頁面管理、註解、表單）、§6「本地崩潰復原」；ADR 0008（worker 隔離）；README：編輯要先做 POC 並寫 ADR
- **負責角色**：核心 agent
- **相依**：無（DEC-03 對 ADR 0010 的決定會影響「存檔時是否寫入文件識別碼」一節）

## 目標
之後所有會修改 PDF 的卡片（B2-02～B2-09、B2-13）共用同一套做法：編輯怎麼表示、在哪裡套用、怎麼復原、怎麼安全地寫回檔案。先寫成 ADR，並以最小的 POC 證明整條路徑可行。

## 範圍
ADR 0013（提議中）至少決定以下幾點，每一點寫出選項與建議：

1. **編輯的表示**：前端送型別化的編輯指令（例如 `RotatePages`、`DeletePages`、`AddHighlight`），由 `ipc_contract` 定義並驗證；前端永遠不送任意的 PDF 物件或內容串流。
2. **在哪裡套用**：在該文件自己的 worker 中，對記憶體裡的文件套用（ADR 0012：每份文件一個 worker）。
3. **復原／重做**：例如 worker 保留指令清單、從原始檔重新套用；或以 MuPDF 的 journal（`mupdf` 繫結目前沒有提供，需要評估）。
4. **寫出**：worker 不能開啟任意路徑（ADR 0008）。
   - 建議：主行程在目的地旁建立暫存檔，把**只能寫入**的 handle 複製給 worker（與開檔時的唯讀 handle 相同的模式）；worker 以 `write_to_with_options` 寫入；主行程確認完整後以原子方式取代目的地。
   - 比較：worker 把整個檔案分段回傳給主行程（受 frame 大小限制）。
5. **增量更新與完整重寫**：已簽章的文件以增量更新存檔，保留原有位元組與簽章；其他文件可選擇完整重寫（`garbage`、`clean`）以縮小檔案。
6. **崩潰復原**（規格 §6「每次編輯動作即存」）：日誌放在 app 的本機資料資料夾（與 `recent.json` 相同），內容與清除時機；日誌可能含有使用者輸入的文字，需要說明隱私影響。
7. **與其他機制的關係**：信任（ADR 0002，內容雜湊在存檔後改變）、文件識別碼（ADR 0010）、權限（MVP-19：作者禁止修改時是否允許編輯，比照 Acrobat）。
8. **上限**：輸出檔案大小、單一文件的編輯指令數、復原步數。

POC（在測試中，不做 UI）：
- worker 整合測試：開啟語料 → 旋轉第一頁 → 經由複製進 worker 的寫入 handle 另存 → 重新開啟，第一頁已旋轉、其他頁不變。
- 以 `benign/signed.pdf` 增量存檔：原本的位元組是新檔案的開頭，簽章範圍（`/ByteRange`）仍涵蓋原內容。

## 不做什麼
- 不做任何編輯或存檔的 UI（B2-02 起）。
- 不做崩潰復原的實作（B2-13）。

## 可動的模組
- `docs/adr/0013-*.md`、`docs/adr/README.md`、`docs/architecture/`（新文件）
- `crates/pdf_worker/tests/`（POC 測試）、必要時 `crates/sandbox`／`crates/worker_host` 的寫入 handle 原型

## 驗收情境
- ADR 0013 狀態為「提議中」，每個決定點都有選項、建議與理由。
- POC 測試在 CI 上通過：旋轉後的檔案正確；已簽章文件的增量存檔保留原位元組。
- worker 在 POC 中沒有取得任何路徑，只拿到 handle。

## 必跑測試
- AGENTS.md 指令表全部通過；新增的 POC 測試。

## 資安限制
- 修改 worker 的權限、IPC 協定或 handle 的傳遞方式都需要 `needs-security-review`。
- 寫入 handle 只允許寫入暫存檔，不能讀取或刪除其他檔案；原檔在新檔完整寫好之前不得變動。
