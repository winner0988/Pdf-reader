---
title: "[B2-11] ADR 0015：數位簽章驗證（唯讀，含 POC）"
labels: task,batch-2,area:worker,area:security,agent:core,needs-security-review
---

- **需求 ID**：規格 §3「本地數位簽章」（本卡只做驗證；簽署另開卡）；README：簽章要先做 POC 並寫 ADR
- **負責角色**：核心 agent
- **相依**：無；QA-04 的簽章語料（`benign/signed.pdf`、`benign/signed-docmdp-p1.pdf`）

## 目標
開啟已簽章的文件時，告訴使用者簽章是否有效：文件在簽章後有沒有被修改、簽署者是誰、憑證是否受信任。驗證完全離線。

## 範圍
ADR 0015（提議中）比較並建議：

1. **在哪裡解析**：簽章字典與 CMS 資料來自不受信任的 PDF，解析必須在 worker（沙盒）中進行。
2. **密碼學實作**：
   - MuPDF 的簽章驗證需要 OpenSSL 後端（目前的建置沒有）；
   - Rust 的 RustCrypto 套件（`cms`、`x509-cert`、`rsa`、`p256`、`sha2`）；
   - Windows CryptoAPI（`CryptVerifyMessageSignature`）。
3. **信任的根憑證**：Windows 的憑證存放區、只顯示「自簽／不受信任」、或 Adobe AATL（需要下載清單，不符合離線原則）。
4. **撤銷狀態**：不查 OCSP／CRL（不連網）。只使用文件內嵌的撤銷資訊（DSS、`adbe-revocationInfoArchival`），其他情況明白說明「無法確認撤銷狀態」。
5. **呈現**：簽章面板或橫幅的狀態種類（有效、文件在簽章後被修改、簽章無效、無法驗證），以及 DocMDP（QA-04 的語料 `signed-docmdp-p1.pdf`：簽署者禁止任何修改）的意義。

POC（測試中）：
- 驗證語料的兩個簽章檔都是「文件未被修改、簽章數學上有效、憑證自簽」；
- 修改一個位元組後變成「文件在簽章後被修改」。

## 不做什麼
- 簽署文件、載入 `.pfx`（另開卡）；時間戳記伺服器（需要連網）。
- UI（ADR 接受後另開卡）。

## 可動的模組
- `docs/adr/0015-*.md`、`docs/adr/README.md`
- POC：`crates/pdf_worker/tests/` 或獨立的 POC crate；`tests/corpus/`（被竄改的簽章樣本由腳本產生）

## 驗收情境
- ADR 0015 為「提議中」，每個決定點都有選項、建議與理由。
- POC 在 CI 上：兩個簽章樣本驗證為有效（自簽）；竄改後的樣本被偵測出來。

## 必跑測試
- AGENTS.md 指令表；POC 測試。

## 資安限制
- 新增密碼學依賴需要 `needs-security-review`，並說明來源與維護狀態。
- 驗證不得連網；不得把「數學上有效」說成「可信任」。
