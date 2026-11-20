# 在真正的 app 裡驗證改動（給所有 agent）

測試通過，不代表 app 真的對了。改到畫面、IPC 或 worker 的行為時，說「做完了」之前，自己建出 app、打開 `tests/corpus/` 的檔案、像使用者一樣操作一次，再把看到的結果寫進 PR。

- 這份文件給所有 agent（Claude、Codex、Antigravity 等），也給人。
- Claude Code 的 `.claude/skills/verify/` 只是指向這裡的入口。改做法時只改這份文件。
- E2E 的運作方式見 [e2e.md](../architecture/e2e.md)；審查別人的 PR 見 [independent-review.md](independent-review.md)。

## 什麼時候要做

| 改到 | 要不要在 app 裡看 |
|---|---|
| 畫面：元件、樣式、對話框、選單、焦點、鍵盤 | 要 |
| IPC：`src-tauri` 的命令、`crates/ipc_contract` 的訊息 | 要 |
| worker 的行為：渲染、文字、搜尋、安全報告、存檔 | 要；可以只問 worker（第 4 步） |
| 只有文件、CI 設定，或單元測試已經直接涵蓋的純邏輯 | 不用 |

審查別人的 PR 時，看不出改動在 app 裡會怎樣，也照這份文件看一次。

## 不能做

這些規則保護使用者的電腦與資料，沒有例外：

1. **不開真正的外部連結，也不讓 app 連網。**
   - 連結確認對話框只檢查它顯示的網站與完整網址，然後按「取消」。不按「開啟」，也不按「複製連結」（會寫進剪貼簿）。
   - 不按設定裡的「檢查更新」：它會連到 GitHub（[update-check.md](../architecture/update-check.md)）。E2E 也從來不按它。
2. **不截整個桌面。** 只用 Playwright 的 `page.screenshot()`：它透過 CDP，只截 WebView 裡的畫面。不用作業系統的截圖功能，也不用截整個螢幕的工具。harness 只在 CI 上才會截整個桌面，所以不要設定 `CI`（第 6 條）。
3. **不讀寫剪貼簿。** 要測「複製」時，照 `tests/e2e/text.spec.ts` 的做法：先把頁面的 `navigator.clipboard.writeText` 換成記錄器，再讀記錄器拿到的文字。不讀系統的剪貼簿，也不看剪貼簿歷程。
4. **只開 `tests/corpus/` 的檔案。**
   - 不開使用者的文件、下載的檔案，或其他語料庫的 PDF。
   - 驗證時 app 從語料檔另存到暫存資料夾的副本，可以再開（E2E 也這樣做）。
   - 需要新的樣本，就修改 `tests/corpus/generate.py`（見 [tests/corpus/README.md](../../tests/corpus/README.md)）。不要手做或下載 PDF。
5. **不操作桌面上的其他 app。** 只透過 CDP 操作這個 app 的 WebView。
   - app 自己的系統對話框與視窗，只用 harness 的 `answerFileDialog`、`closeMessageBox`、`closeAppWindow`。它們只找 app 行程自己的視窗。
   - 不用模擬滑鼠鍵盤或桌面自動化工具去碰其他視窗。
6. **不改這台電腦的設定。**
   - 執行 E2E 或 `cargo test` 之前，在要執行的那個 shell 裡確認下面的環境變數都沒有設定（建置不讀它們）。它們讓測試做只該在用完即丟的 CI runner 上做的事：
     - `CI`：harness 寫入整台電腦的 WebView2 原則（HKLM），WebView 沒有啟動時改截整個桌面（[e2e.md](../architecture/e2e.md)）；下面幾項的測試也都會跑；
     - `E2E_SIGNING`、`PDF_READER_TEST_SIGNING`：把測試憑證放進使用者的憑證存放區（[signing.md](../architecture/signing.md)）；
     - `E2E_REGISTER_FONT`：為目前的使用者登錄一個字型（[text-boxes.md](../architecture/text-boxes.md)）；
     - `PDF_READER_TEST_CLIPBOARD`：換掉剪貼簿上的內容。
   - 不自己設定 WebView2 的環境變數或登錄機碼來開除錯埠。
   - 不直接執行 `pdf-reader.exe` 來驗證：那會用到使用者自己的設定與最近開啟的檔案。
   - 用 harness 的 `launch`。它每次啟動都：
     - 只對這一次啟動開除錯埠（只綁 127.0.0.1）；
     - 用新的暫存 WebView2 設定與資料資料夾；
     - 在結束時刪掉它們。
7. **開 app 之前先問。** app 會在桌面上開真的視窗，有些測試還會回答系統的對話框。在有人正在使用的電腦上，先問過對方。執行期間不要操作滑鼠鍵盤（[e2e.md](../architecture/e2e.md)）。
8. **不把私人或危險的東西放進 PR。**
   - 截圖只能是語料檔的畫面。
   - 不貼本機路徑、使用者名稱，也不貼 WebView2 的整份網路記錄（見「證據」）。
   - 讓 worker 當掉或卡住的檔案是安全樣本：不放進 Issue、PR 或任何公開的地方，照 [SECURITY.md](../../SECURITY.md) 私下回報。
9. **不為了好測而改 app。** 不打開開發人員工具，不放寬 CSP 或 capability，不加測試專用的建置選項。app 沒有為測試做任何修改（[e2e.md](../architecture/e2e.md)），驗證的就是出貨的樣子。

[manual-checks.md](../security/manual-checks.md) 的人工安全檢查，要由人在測試用的電腦上做。裡面有按「開啟」、截整個螢幕、看剪貼簿歷程、用系統管理員權限執行的工具等步驟，agent 都不代做；這份文件也不取代那些檢查。

## 怎麼做

### 1. 準備 worktree 與短的建置資料夾

- 在自己分支的 worktree 裡做，不在共用的工作目錄動手（那裡可能有別人未提交的變更）。
- Windows 的路徑有 260 字元的限制（見「常見問題」）：
  - 建置資料夾（`target/`，或環境變數 `CARGO_TARGET_DIR` 指定的資料夾）的路徑太長，MuPDF 的建置會失敗。實測：約 50 個字元的 `target` 可以建置，超過 100 個字元曾經失敗；
  - worktree 的路徑太長，vitest 會失敗（`node_modules` 裡的路徑很深）。
- 能自己選位置時，把 worktree 開在路徑短的資料夾，例如磁碟根目錄下一兩層，或系統的暫存資料夾底下。在 repo 或它的任一個 worktree 裡執行：

```
git worktree add <短路徑> <分支>
```

- 那個分支已經在別的 worktree 裡時，git 不准再 check out 一次。改開 detached 的 worktree：

```
git worktree add --detach <短路徑> <分支>
```

- 改動仍在原本的 worktree 提交。提交之後，讓短路徑的 worktree 切到新的 commit，再建置一次：

```
git -C <短路徑> checkout --detach <分支>
```

- 已經在很深的 worktree 裡（例如工具自動建立的），而且只做 E2E 時：把 `CARGO_TARGET_DIR` 設到短的資料夾，執行 E2E 時再用 `E2E_APP` 指向那裡的 app（第 2 步）。
- 沒有要用 `CARGO_TARGET_DIR` 時，確認它沒有被設定：設了的話，建置會放到那個資料夾，不在 worktree 的 `target/`（共用的建置資料夾見「常見問題」）。
- 記下這時的 `git status`：驗證完要回到一樣的狀態（第 6 步）。

在 worktree 裡安裝依賴（在 PowerShell 執行，見「常見問題」）：

```
pnpm install --frozen-lockfile
```

### 2. 建置 app

```
pnpm e2e:build
```

- 建出 release 版的 `pdf_worker` 與 app：`target/release/pdf-reader.exe`，旁邊是 `pdf_worker.exe`。不建安裝檔。
- 在 PowerShell 或 Git Bash 執行都可以；只有 `pnpm install` 要在 PowerShell。
- 新的 worktree 第一次建置，要編譯 MuPDF 與所有的 Rust 依賴：
  - 試用時約 11 分鐘（worker 與 MuPDF 約 4 分鐘，其餘是 app），`target` 約 4.3 GB；
  - 之後是增量建置；
  - agent 的指令有逾時限制時，在背景執行，等它結束再繼續。
- E2E 跑的是建好的執行檔：改了前端或 Rust 之後，要再 `pnpm e2e:build` 一次，否則看到的是舊的 app。
- 不用 `pnpm tauri dev` 驗證：開發模式的頁面由 Vite 提供，CSP 也不同（`vite.config.ts`），有些問題只在 release 建置出現。
- 設了 `CARGO_TARGET_DIR` 時，app 在那個資料夾的 `release/pdf-reader.exe`。執行 E2E 時設定環境變數 `E2E_APP=<pdf-reader.exe 的路徑>`（[e2e.md](../architecture/e2e.md)）；其他位置建好的 app 也一樣。

### 3. 在 app 裡操作（畫面與 IPC）

在 `tests/e2e/` 加一個臨時的 spec，名稱用 `zz-verify-<主題>.spec.ts`。它用 harness（`tests/e2e/app.ts`）啟動 app，以 CDP 連上 WebView2：

```ts
// tests/e2e/zz-verify-<topic>.spec.ts (scratch: delete it afterwards, never commit it)
import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("<what you check>", async ({ launch }) => {
  const page = await launch(corpus("benign/multi-page-10.pdf"));
  // Page 1 is rendered by the sandboxed worker, not just laid out.
  await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute("data-state", "ready");

  // Act like a user here: roles and names come from src/i18n/zh-TW.ts, never typed in.

  // The WebView only (CDP), into this test's output folder.
  await page.screenshot({ path: test.info().outputPath("verify.png") });
});
```

執行：

```
pnpm e2e zz-verify-<主題>.spec.ts
```

- **等畫好再看**：第 1 頁的 `data-state` 變成 `ready`，再操作或截圖。
- **步驟從既有的 spec 抄**：那個功能在 `tests/e2e/*.spec.ts` 已有的測試，知道選單的名稱與要等什麼。
- **截圖**：在 `tests/e2e/test-results/` 底下這個測試的資料夾。
  - 自己打開看一次，確定截到的是要確認的畫面。
  - 下一次執行 E2E 會清掉這個資料夾；要留就先複製到 repo 以外。
- **直接呼叫主行程的命令**：照 `tests/e2e/saving.spec.ts`，在 `page.evaluate` 裡呼叫 `window.__TAURI_INTERNALS__.invoke("<命令>", <參數>)`。
  - 參數照 `src-tauri` 裡那個命令的定義；文件代號取自頁面的 `data-doc`。
- **讓某個命令晚一點回應**（重現時序的問題）：包住頁面的 `window.fetch`。
  - Tauri 的 IPC 是對 `http://ipc.localhost/<命令>` 的 `fetch`，每次呼叫時才去找 `fetch`。
  - `__TAURI_INTERNALS__.invoke` 本身改寫不了。
- **確認沒有連網**：只改到連網相關的程式時，直接跑 `pnpm e2e network.spec.ts`。要在自己的情境裡看，照 `tests/e2e/network.spec.ts`：
  - 以 `launch(file, { netLog })` 啟動，記錄檔放在系統暫存資料夾，檔名要短；
  - 用 `quit(page)` 結束 app 之後再讀記錄；
  - 用同一個檔案的 `external()` 列出 app 自己的內容以外的請求；
  - 用完刪掉記錄檔。
- **失敗時**：harness 把截圖、app 的日誌與摘要寫到 `tests/e2e/test-results/`；報告在 `tests/e2e/playwright-report/index.html`。

### 4. 直接問 worker（渲染、文字、安全報告）

不需要視窗。照 `crates/pdf_worker/tests/properties.rs` 的 `test_ipc_worker_host_properties_and_fonts` 寫一個 Rust 測試。`WorkerHost` 用和 app 相同的沙盒與 IPC 啟動真正的 `pdf_worker`；測試開一個語料檔，送一個 `WorkerRequest`，再檢查回覆：

```rust
let mut host = WorkerHost::new(Path::new(env!("CARGO_BIN_EXE_pdf_worker")), HostConfig::default());
let (doc, _) = host.open(&corpus("benign/metadata-full.pdf")).expect("open");
let reply = host
    .request(|request| WorkerRequest::GetDocumentProperties { request, doc })
    .expect("request");
```

執行：

```
cargo test --workspace --locked <測試名稱>
```

這個行為要一直成立，就把測試留下來；只是確認一次，就刪掉。

### 5. 證據

寫在 PR 說明的「如何驗證」（PR 模板），讓審查者照著就能重做：

- 驗證的 commit；
- 開的語料檔（`tests/corpus/` 底下的相對路徑）；
- 每一步做了什麼、看到什麼：頁面上的文字、狀態列、worker 的回覆、截圖上的內容；
- 沒有確認到的部分，與原因（例如要 CI 上才有的測試憑證，或要人做的安全檢查）。

截圖：

- 只截 WebView（`page.screenshot()`），內容只有語料檔。
- 要放進 PR，從 GitHub 網頁上 PR 說明或留言的編輯框上傳。不能上傳時，在說明寫出截圖上看到什麼。
- 檔名與畫面上都不能有本機路徑或使用者名稱。

網路記錄：

- WebView2 的網路記錄裡有這台電腦的路徑與命令列，不要整份貼上或上傳。
- 貼 `external()` 列出的結果（應該是空的），並寫出 app 開了多久、做了哪些操作。

發現問題時照實寫，附上重現步驟；不要說做完了。

### 6. 收尾

- 刪掉臨時的 spec，以及只為了確認一次而寫的 Rust 測試。
- `git status` 要和第 1 步記下的一樣。`target/`、`node_modules/`、`tests/e2e/test-results/`、`tests/e2e/playwright-report/` 都被 `.gitignore` 忽略，不會出現在裡面。
- 要留的截圖，先複製到 repo 以外（第 3 步）。
- 只為了驗證而另外開的 worktree，用完就移除：它的 `target/` 有好幾 GB。在別的 worktree 裡執行：

```
git worktree remove <短路徑>
```

## 什麼時候該寫 E2E 測試

只看一次的確認，下一個改動就可能弄壞而沒人發現。以下情況，把臨時的 spec 改成正式的測試：

- 工作卡的驗收情境，或修好的 bug 不能再出現；
- 只有真正的 app 才看得到的行為：排版與位置（jsdom 不排版）、焦點與鍵盤、主行程與 worker 之間、沙盒、CSP、WebView2、系統對話框；
- 修正：先確認拿掉修正時，測試真的會失敗（[independent-review.md](independent-review.md) 第 4 步）。

正式的測試用一般的檔名（不用 `zz-verify-`），並在 [e2e.md](../architecture/e2e.md) 的「目前的測試」表加一列。寫法照 e2e.md：預期文字從 `src/i18n/zh-TW.ts` 取得，不靠重試（`retries: 0`），不穩定是要修的錯誤。

不用寫成 E2E 的：

- 元件的細節：用 Vitest（`pnpm test`），比較快；
- worker 自己的行為：用 Rust 測試（第 4 步）；
- 審查時看一眼畫面的確認：寫在審查留言裡就好。

## 常見問題

| 症狀 | 原因與做法 |
|---|---|
| MuPDF 的建置失敗：`msbuild invocation failed` | 建置資料夾的路徑太長。把 worktree 移到短路徑，或把 `CARGO_TARGET_DIR` 設到短的資料夾，再用 `E2E_APP` 指向那裡的 `release/pdf-reader.exe` |
| vitest 一啟動就失敗（`ERR_PACKAGE_IMPORT_NOT_DEFINED`） | worktree 的路徑太長，`node_modules` 裡的路徑超過 260 字元。把 worktree 移到短路徑 |
| tsc、vitest、eslint 找不到模組 | 曾發生在 Git Bash 裡執行 `pnpm install` 之後：`node_modules` 連到 Windows 的 node 讀不到的路徑。刪掉 `node_modules`，改從 PowerShell 安裝 |
| E2E 看到的還是舊的行為 | 改完之後沒有再 `pnpm e2e:build` |
| `pnpm test` 有幾個測試逾時，單獨跑又會過 | 和建置（cargo）同時跑，CPU 被搶。等建置結束再跑前端測試 |
| 幾個 worktree 共用一個 `CARGO_TARGET_DIR`，結果說不通 | Cargo 把原始檔比上次建置還舊的 crate 當成最新，拿到別的分支建的版本。每個 worktree 用自己的 target；要共用，就在建置前更新這個分支 Rust 原始檔的修改時間，而且一次只跑一個建置 |
| 對話框開著時，`getByRole` 找不到頁面 | modal 對話框把 `#root` 設成 `aria-hidden`。在開對話框之前讀，或改用 CSS 定位器 |
| 包住 `__TAURI_INTERNALS__.invoke` 沒有作用 | 它改寫不了。改包頁面的 `window.fetch`（第 3 步） |
| app 的頁面很久才出現 | 全新的 WebView2 設定有時很晚才載入 app 的頁面（CI 上曾要 17～35 秒）。harness 最多等 60 秒，超過 5 秒時會印出花了多久與當時的情況（[e2e.md](../architecture/e2e.md)） |
