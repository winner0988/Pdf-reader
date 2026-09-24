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
| `benign/` | 一般文件：單頁、多頁、中英文、三層目錄、內部連結、外部連結、旋轉頁、不同尺寸、純圖片；加密（RC4 40-bit、AES-256）；已簽章、DocMDP 認證簽章 | 功能測試（MVP-06～12）；ADR 0006、ADR 0010 的測試 |
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
- **AES-256（R6）**：
  - 產生器自己的 AES 每次執行都先以 FIPS-197 的測試向量自我檢查（`check_aes`）；
  - `crates/pdf_worker/tests/corpus_crypto.rs` 以 MuPDF 確認兩個加密樣本都需要密碼、拒絕錯誤的密碼，並能以使用者密碼與擁有者密碼解密出正確文字；worker 本身則回報「加密，不支援」。
- **簽章**：
  - 兩個簽章樣本以 OpenSSL（`openssl cms -verify -noverify`）驗證成功，驗證的內容正好是 `/ByteRange` 涵蓋的位元組；
  - 改動其中一個位元組後驗證失敗；
  - 同一個 Rust 測試確認 MuPDF 讀得到簽章欄位、`/ByteRange` 涵蓋 `/Contents` 以外的整個檔案，以及 DocMDP 的 `P 1`。

## 測試用的簽章金鑰

- `signed.pdf` 與 `signed-docmdp-p1.pdf` 以 RSA-2048 的測試金鑰簽章（SHA-256，`adbe.pkcs7.detached`），憑證是自簽的：
  - 主體為 `CN=PDF Reader test corpus signer (NOT TRUSTED)`；
  - 有效期間 2026-01-01～2036-01-01。
- 金鑰在每次產生時由 `generate.py` 以固定的種子推導，**repo 中沒有儲存任何金鑰**。
- 任何人都能以同樣方式重建它，所以它只能用於這個語料庫：**不得信任這張憑證，也不得把這把金鑰用在任何其他地方。**

## 注意

- `mixed-text-zh-en.pdf` 的中文字型沒有嵌入。worker 以內建的 Droid CJK 字型顯示它（DEC-03）；沒有 CJK 備援字型的引擎無法載入這個字型，中文既畫不出來也搜尋不到。
