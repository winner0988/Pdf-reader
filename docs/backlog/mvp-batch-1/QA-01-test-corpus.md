---
title: "[QA-01] 測試 PDF 語料庫：良性、惡意（無害化）、損毀、超大"
labels: task,mvp,qa,area:security,agent:qa
---

- **需求 ID**：MVP-R1～R10 的測試基礎；ADR 0008（fuzzing 種子）
- **負責角色**：QA／安全 agent
- **相依**：無（可立即開始）

## 目標
建立一份可重現、無害化、授權乾淨的測試 PDF 語料庫，供單元測試、E2E、手動安全驗證與 fuzzing 使用。

## 範圍
- `tests/corpus/` 結構：`benign/`、`malicious/`、`malformed/`、`large/`（`large/` 只提交產生腳本，不提交大檔）。
- 產生器腳本：優先使用 Python 標準函式庫直接寫出 PDF 物件；相同輸入必須產生相同位元組。
- `tests/corpus/manifest.json`：每個檔案的用途、預期頁數、預期安全發現、預期行為。
- **benign**：單頁、多頁、中英文文字、三層目錄、內部連結、外部 https 連結、`/Rotate 90` 頁面、不同頁面尺寸、無文字層的圖片型頁面。
- **malicious**（全部無害化）：文件層 JavaScript、OpenAction JavaScript、OpenAction URI、頁面 `/AA`、表單欄位 `/AA`、`/Launch`、`/SubmitForm`、指向 UNC 路徑的 `/GoToR`、遠端 URL 檔案規格、XFA、`javascript:`／`file:`／`smb:` 連結、IDN 同形異義網域連結、含 RTL 覆寫字元的連結、10,000 字元的 URL。
- **malformed**：截斷檔、xref 損毀、物件循環參照、目錄循環、宣告超大頁面尺寸、深層巢狀、10 萬個目錄項目、改副檔名的非 PDF 檔。
- **large**：1000 頁、約 200 MB 的產生腳本（供 MVP-07 效能驗收）。
- 加密（AES-256 開啟密碼）與已簽章範例：若需要第三方工具（如 qpdf、mutool），在 PR 中註明工具來源與版本。

## 不做什麼
- 不從網路下載任何惡意樣本或公開惡意 PDF 資料集。
- 不使用任何真實個人資料或私人文件。

## 可動的模組
- `tests/corpus/`
- `.github/workflows/ci.yml`（加入「重新產生後與提交內容一致」的檢查）

## 驗收情境
- 在乾淨環境執行產生指令（例如 `python tests/corpus/generate.py`）可產生全部檔案；重跑後位元組完全相同，CI 驗證這一點。
- `manifest.json` 涵蓋上述每一類樣本。
- 所有惡意樣本的目標都是 `.invalid` 網域（例如 `https://beacon.example.invalid/`、`\\share.example.invalid\x`）或不存在的程式名稱，不可能真的連線或執行。

## 必跑測試
- 產生器可重現性檢查（CI）。
- `bash scripts/ci/check-forbidden.sh` 仍通過。

## 資安限制
- 樣本只能「宣告」危險動作，不得包含真實 exploit、shellcode 或可運作的惡意程式碼。
- 此卡修改 CI，需加 `needs-security-review`。
