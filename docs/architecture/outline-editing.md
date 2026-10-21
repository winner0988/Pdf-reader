# 書籤與目錄編輯（Outline Editing）

本文件說明 PDF 書籤（目錄項目，Outline Item）的新增、重新命名、移動、刪除與一致性維護機制（Issue #305，B3-17）。

---

## 1. 書籤識別方式（Identification）

### 問題：為何不能使用位置路徑？
前端在展示目錄樹時，常以預序遍歷清單或樹狀路徑（例如 `[2, 0, 3]`）定位項目。然而在編輯過程中：
1. **路徑脆弱**：新增、刪除或移動同層或祖先節點時，後續所有項目的路徑索引都會位移。
2. **復原重做（Undo/Redo）與崩潰復原重播（Crash Recovery Replay）失效**：若日記（Journal）中記錄的是路徑，在依序重播一系列編輯動作時，先前的動作可能改變了樹狀結構，導致後續動作指向錯誤節點。

### 解決方案：間接物件編號 `BookmarkId(u32)`
每個 PDF outline item 在 PDF 規範中都是一個間接字典物件（Indirect Object Dictionary）。
- 我們將 `BookmarkId(pub u32)` 定義為該項目在 PDF 中的間接物件編號（Object Number / XRef）。
- **穩定性**：既有物件的物件編號在編輯過程中固定不變；由日誌重新建立時，MuPDF 的物件編號分配遵循確定性順序。
- **邊界傳輸**：`BookmarkId` 透過 IPC 與前端同步，在 `OutlineItem.id` 中提供給前端。前端發送編輯指令（`Edit::AddBookmark`、`RenameBookmark`、`DeleteBookmark`、`MoveBookmark`）時皆使用 `BookmarkId`。

---

## 2. 結構一致性規則（Consistency Rules）

PDF 規格（ISO 32000-1 §12.3.3）定義目錄樹為雙向鏈結串列（Doubly-linked list），具有階層指標：
- 每個 Outline Item 包含：`/Title`、`/Dest`（或 `/A`）、`/Parent`、`/Prev`、`/Next`、可選的 `/First`、`/Last`、`/Count`。
- 目錄根節點（Catalog 的 `/Outlines` 字典）包含：`/Type /Outlines`、`/First`、`/Last`、可選的 `/Count`。

### 編輯一致性維護
在 worker 的 `crates/pdf_worker/src/engine/outline.rs` 中：
1. **建立根節點**：若文件原本沒有目錄，新增第一個書籤時自動在 Catalog 中建立 `/Outlines` 字典。
2. **雙向鏈結與父子指標**：透過 `set_children` 函式集中更新給定父節點的所有子項目：
   - 第一個項目的 `/Prev` 刪除，最後一個項目的 `/Next` 刪除。
   - 所有中間項目的 `/Prev` 與 `/Next` 指向其兄弟。
   - 所有子項目的 `/Parent` 指向父節點。
   - 父節點的 `/First` 與 `/Last` 指向首末子節點。
   - 若父節點已無子項目，刪除其 `/First`、`/Last` 與 `/Count`。
3. **`/Count` 計數與開合狀態一致性**：
   - 規格定義：若項目收合（Closed），`/Count` 為負值，其絕對值為展開時該子樹所含的總可見子項目數；若展開（Open），`/Count` 為正值。
   - 根節點的 `/Count` 永遠為非負整數，代表目前整個目錄中可見的項目總數。
   - 每次增刪改或移動後，`recompute_all_counts` 遞迴重新計算整棵樹的 `/Count`，嚴格保留使用者原有的開合正負號。
4. **目標定位**：
   - 新增的書籤目標皆為顯式頁面跳轉目標（Explicit Destination）：`[page /XYZ 0 height 0]`，跳轉至該頁左上角。
   - 不建立任何 `/A` 動作字典或未知的 JavaScript / 外部啟動，符合安全規格（ADR 0002）。

---

## 3. 上限與驗證（Limits & Validation）

為防止惡意構造的 PDF 造成記憶體耗盡或遞迴堆疊溢位（Stack Overflow），設定以下硬性上限：

| 常數 | 上限值 | 理由與依據 |
|---|---|---|
| `MAX_TEXT_BYTES` | 1,024 bytes | 與既有書籤讀取、註解標題長度上限一致，避免超長字串消耗版面與 IPC 資源。 |
| `MAX_OUTLINE_DEPTH` | 64 | 與 `ipc_contract::limits::MAX_OUTLINE_DEPTH` 一致，防止超深巢狀目錄導致排版或遞迴溢位。 |
| `MAX_OUTLINE_ITEMS` | 10,000 | 與 `ipc_contract::limits::MAX_OUTLINE_ITEMS` 一致，防止極大量項目導致效能降低。 |

所有編輯操作在 `ipc_contract::validate` 與 worker `pdf_worker::engine::outline` 雙重驗證：
- 標題去除控制字元與前後空白，不得為空。
- 移動節點時進行環狀依賴檢查（Cycle Prevention）：禁止將任何項目移動到自身或其子孫節點之下。

---

## 4. 復原、重做與崩潰復原（Undo, Redo, Crash Recovery）

依據 ADR 0013：
- 每個書籤編輯動作對應一個型別化的 `Edit` 變體：
  - `Edit::AddBookmark { parent: Option<BookmarkId>, index: u32, title: String, page: u32 }`
  - `Edit::RenameBookmark { id: BookmarkId, title: String }`
  - `Edit::DeleteBookmark { id: BookmarkId }`
  - `Edit::MoveBookmark { id: BookmarkId, parent: Option<BookmarkId>, index: u32 }`
- 編輯動作記錄於崩潰復原日誌（Crash Recovery Journal）。
- 執行復原時，`WorkerRequest::Revert` 從原始 PDF 位元組重新開啟，並重播日誌中的編輯動作清單；由於動作以物件編號與確定性順序表示，重播後目錄結構與狀態維持百分之百一致。
