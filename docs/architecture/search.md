# 全文搜尋

對應工作卡 MVP-10。本文件說明主行程與 worker 端的搜尋（10a）；搜尋列與高亮見 10b。

## 流程

```mermaid
sequenceDiagram
  participant UI as 前端
  participant S as 主行程：search::run
  participant R as 主行程：Renderer 執行緒
  participant W as pdf_worker
  UI->>S: search(args, onEvent)
  loop 每一頁（直到取消、文件關閉或達上限）
    S->>R: 背景工作：搜尋第 n 頁（有渲染請求時先渲染）
    R->>W: SearchPage { page_index, query, case_sensitive, max_hits }
    W-->>R: PageSearched { hits, has_text }
    R-->>S: 結果
    S-->>UI: hits（有命中時）／progress（每 100 ms）
  end
  S-->>UI: done { totalHits, truncated, noTextLayer }
```

- **逐頁搜尋，由主行程驅動**：worker 一次只處理一個請求。若用單一個長時間的搜尋請求，搜尋期間無法渲染頁面，也不容易取消。所以主行程一頁一頁送 `SearchPage`，每頁是一個獨立請求。
- **渲染優先**：每一頁的搜尋以「背景工作」排在渲染執行緒上（`Renderer::in_background`），只有在沒有等待中的渲染請求時才執行。搜尋長文件時，捲動與縮放仍然順暢。
- **取消**：`cancel(request)` 設定該次搜尋的旗標，搜尋在下一頁之前停止（`Searches`）。前端開始新搜尋、修改關鍵字或關閉搜尋列時會取消舊的搜尋，並忽略舊搜尋的所有結果。文件關閉時，下一頁的搜尋會以 `unknownDocument` 結束。
- **上限**：查詢最長 `MAX_QUERY_BYTES`（1,024 bytes）；整份文件最多 `MAX_SEARCH_HITS`（10,000）筆，達到時停止並標記 `truncated`；每筆命中最多 `MAX_QUADS_PER_HIT`（64）個區塊。worker 每頁最多只回傳「剩餘可用」的筆數。
- **沒有文字層**：所有頁面都沒有任何文字時，`done.noTextLayer = true`，前端提示「此文件沒有文字層，目前版本尚不支援 OCR」。

## 比對方式（`crates/pdf_worker/src/search.rs`）

- 從 MuPDF 的文字層（structured text）逐行取出字元與其四邊形位置。
- 比對前「摺疊」：
  - 各種空白與控制字元都當成一個空格，連續空格合併；
  - 行與行之間以一個空格相接，片語可以跨行；
  - 不分大小寫時轉成小寫（只用一對一的對應，確保字元與位置對得上）。
  - 查詢字串用同樣方式處理，前後空白去掉。
- 以 **KMP** 比對，時間與頁面文字量成線性：惡意 PDF 放入大量文字加上長查詢，也不會變成平方時間（測試：100 萬字元的頁面、1,000 字元的查詢，2 秒內完成）。
- 命中不重疊。每筆命中在每一行產生一個四邊形，從該行第一個字元到最後一個字元。座標是頁面座標（point，原點在左上），與頁面尺寸一致；縮放與旋轉由前端換算。
- 中文不需要特別處理：比對的是文字層的 Unicode。

## 限制：未內嵌的 CJK 字型

worker 的 MuPDF 不含 CJK 字型（見 [mupdf-binding.md](mupdf-binding.md)）。遇到**未內嵌**的 CJK Type0 字型時，MuPDF 無法載入字型，連字碼解碼也一併失去，文字層變成亂碼，所以搜尋不到。QA-01 的 `benign/mixed-text-zh-en.pdf` 就是這種檔案。

有內嵌字型、或有 ToUnicode 的文件，中文搜尋正常（`searches_chinese_text_from_the_to_unicode_map`）。字型來源是 DEC-03（#31）的決策；決定後補上這個檔案的端對端測試。

## 量測

`crates/worker_host/src/bin/search_bench.rs` 在沙盒中開啟文件，以主行程相同的方式逐頁搜尋。

```bash
cargo build --release -p pdf_worker -p worker_host
target/release/search_bench.exe tests/corpus/large/output/large-1000-pages.pdf privacy
```

開發者電腦（Windows 11）、release 建置、QA-01 `large-1000-pages.pdf`（197 MB，每頁一行可搜尋的文字）：

| 查詢 | 命中 | 第一筆 | 前 500 頁 | 全部 1000 頁 | 預算 |
|---|---|---|---|---|---|
| `privacy`（每頁都有） | 1000 | 4 ms | 75 ms | 145 ms | 第一筆 < 1 s、500 頁 < 5 s |
| `needle-0500`（只在第 500 頁） | 1 | 68 ms | 68 ms | 143 ms | |
| `PRIVACY`（區分大小寫） | 0 | — | 61 ms | 122 ms | |

以上是 worker 端的時間。經過主行程排程與前端之後的時間在 10b 量測。
