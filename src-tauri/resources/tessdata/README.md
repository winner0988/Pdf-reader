# 內附的 OCR 語言資料

[ADR 0015](../../../docs/adr/0015-ocr.md) 決定的 OCR（Tesseract，只用 LSTM）需要語言資料。這兩份隨安裝檔附上，由主行程讀取、以位元組交給 worker；app 不下載任何東西。

| 檔案 | 大小 | SHA-256 |
|---|---|---|
| `eng.traineddata` | 4,113,088 位元組 | `7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2` |
| `chi_tra.traineddata` | 2,366,642 位元組 | `529c5b5797d64b126065cd55f2bb4c7fd7b15790798091b1ff259941a829330b` |

- **來源**：[tesseract-ocr/tessdata_fast](https://github.com/tesseract-ocr/tessdata_fast)（較小的「fast」LSTM 模型）的 commit `87416418657359cb625c412a48b6e1d6d41c29bd`。
- **授權**：Apache License 2.0（見 tessdata_fast 的 `LICENSE`），與本專案的 AGPL-3.0-or-later 相容。這兩個檔案沒有修改。
- **放進 repo 的原因**：負責人 2026-10-07 的決定：CI 與安裝檔的建置不需要任何下載，repo 多 6.5 MB。
- 測試檢查這兩個檔案的大小與雜湊值，也檢查格式（`crates/ipc_contract/src/ocr.rs`），所以換掉它們必須同時改這裡的說明與測試。`.gitattributes` 把 `*.traineddata` 當成二進位檔，換行字元不會被改動。
