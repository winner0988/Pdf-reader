# 測試 PDF 語料庫

所有檔案都由 [`generate.py`](generate.py)（只用 Python 標準函式庫）產生。**不要手動修改或新增 PDF**；要新增樣本就修改產生器。

```bash
python tests/corpus/generate.py           # 重新產生已提交的檔案與 manifest.json
python tests/corpus/generate.py --large   # 另外產生大型檔案到 large/output/（不提交）
```

輸出是位元組層級可重現的：不壓縮、不用亂數、沒有時間戳，在任何平台與 Python 3.9+ 結果都相同。CI 的 `Guardrails` 會重新產生並比對，不一致就失敗。

## 安全規則

- **惡意樣本只「宣告」危險動作，不含任何 exploit 或 shellcode。** 目標一律是 `.invalid` 網域（RFC 2606，永遠無法解析）或不存在的程式名稱，例如 `https://beacon.example.invalid/`、`\\share.example.invalid\x`、`does-not-exist.example.exe`。
- 不從網路下載任何樣本，不使用任何真實文件或個人資料。
- 即使如此，也**不要**用其他 PDF 閱讀器開啟 `malicious/` 中的檔案做實驗；請用本專案的應用程式或測試。

## 分類

| 目錄 | 內容 | 用途 |
|---|---|---|
| `benign/` | 一般文件：單頁、多頁、中英文、三層目錄、內部連結、外部連結、旋轉頁、不同尺寸、純圖片、加密 | 功能測試（MVP-06～12） |
| `malicious/` | 主動內容與遠端引用：JavaScript、OpenAction、AA、Launch、SubmitForm、ImportData、GoToR（UNC）、GoToE、遠端檔案規格、XFA、嵌入檔案；危險 scheme 與偽裝連結 | 封鎖與偵測（MVP-11、MVP-12） |
| `malformed/` | 截斷、xref 錯誤、頁面樹循環、目錄循環、超大頁面、深層巢狀、零頁、非 PDF、空檔 | 錯誤處理、fuzzing 種子（QA-03） |
| `large/output/` | 1000 頁約 200 MB、10 萬個目錄項目（`--large` 產生，不提交） | 效能與上限（MVP-07、09、10） |

每個檔案的用途、預期頁數、預期的安全發現、可搜尋文字與預期行為都列在 [`manifest.json`](manifest.json)：

| 欄位 | 說明 |
|---|---|
| `path` | 相對於本目錄 |
| `category` | `benign`／`malicious`／`malformed`／`large` |
| `purpose` | 這個樣本測什麼 |
| `expected` | 應用程式應有的行為 |
| `pages` | 預期頁數；損毀檔為 `null`（依修復結果而定） |
| `findings` | 預期的安全發現，名稱與 `ipc_contract` 的 `FindingKind` 相同（camelCase） |
| `text` | 預期可搜尋到的文字 |
| `bytes`、`sha256` | 已提交檔案的大小與雜湊 |
| `onDemand` | `true` 表示由 `--large` 產生、不在 repo 中 |

## 已驗證

- 所有 benign／malicious 樣本的 xref 位移都指向正確的物件（產生器內建檢查）。
- 以 Chromium 內建的 PDF 檢視器（PDFium，與 MuPDF 無關的另一套引擎）開啟：目錄、中英文字、圖片、不同頁面尺寸、1000 頁大型檔都能正常顯示；加密檔會要求密碼。
- 加密實作（RC4 40-bit，標準安全處理常式 R2）：以相同程式碼產生空白使用者密碼的版本，PDFium 能直接解密並顯示正確文字。

## 尚未涵蓋

- **AES-256 加密**與**已簽章**樣本：需要 AES 與 PKCS#7，標準函式庫無法產生。待 MuPDF 工具（MVP-03）可用後補上，追蹤於另開的 Issue。
- `mixed-text-zh-en.pdf` 的中文字型未嵌入，是否顯示中文字形取決於引擎是否有 CJK 備援字型；可搜尋文字不受影響。
