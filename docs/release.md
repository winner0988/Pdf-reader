# 發行流程

REL-04（[#198](https://github.com/winner0988/Pdf-reader/issues/198)）。怎麼把一個版本交到使用者手上，以及其中哪些是人要按的。

**原則**：發行是對外的、不可逆的（發布後「檢查更新」就會把使用者帶過去），所以 CI 只把一切準備好、做成**草稿**；tag 與「發布」兩個按鈕是專案負責人的，AI agent 不碰。

## 發行時發生什麼

推上形如 `v0.1.0` 的 tag，[`Release` 工作](../.github/workflows/release.yml)依序做：

1. **檢查**（`checks`）：`package.json`、`Cargo.toml`、`Cargo.lock` 是同一個版本，tag 是 `v<那個版本>`，`CHANGELOG.md` 有那個版本的日期小節（[check-version.mjs](../scripts/release/check-version.mjs)）；第三方元件聲明是這個 commit 的依賴會產生的樣子（`third-party-licenses.mjs --check`）；發行腳本的測試。
2. **安裝檔**（`installer`）：就是 [`Installer` 工作](../.github/workflows/installer.yml)，被這個工作呼叫：建置、檢查內容（含 `pdf_worker.exe`、OCR 語言資料、`LICENSE`、`THIRD_PARTY_LICENSES.txt`）、靜默安裝、PDF 關聯、不需要 VC++ 執行階段、安裝後的 worker 在沙盒中跑、**安裝後的 app** 跑一小組 E2E（開檔、授權、不連網、掃描頁辨識）、解除安裝，再留下雜湊值（[packaging.md](architecture/packaging.md)）。
3. **草稿**（`draft`）：只有 tag 才做。下載上一步的安裝檔，確認它的雜湊值還是 Windows 工作算出的那個，改成沒有空白的檔名（GitHub 會把附件名稱裡的空白換成點，`.sha256` 與發行說明裡的檔名就對不上了），重新寫一份 `.sha256`，用 [release-notes.mjs](../scripts/release/release-notes.mjs) 把 `CHANGELOG.md` 的小節加上下載與核對、原始碼與授權寫成發行說明，建立**草稿** Release（`gh release create --draft --verify-tag`）。

草稿只有有權限的人看得到，也不是「最新版本」：[ADR 0009](adr/0009-default-network-policy.md) 的檢查更新讀的是 `releases/latest`，草稿與預先發行版都不算。

在 Actions 頁面手動執行 `Release`（Run workflow）做 1、2，與 3 的前幾步（下載安裝檔、改名、重做 `.sha256`、寫發行說明），只差最後的「建立草稿」：那一步不做，改成列出會附上的檔案，所以不建立 release。推 tag 之前先這樣演練一次，可以先知道會不會失敗。修改 `release.yml` 或 `scripts/release/` 的 PR 會自動執行 1（2 在那種 PR 上由 `Installer` 工作自己做）。

## 負責人的檢查表

第一次發行（0.1.0）與之後每次發行都一樣，打勾的是只有人能做的事。

### 發行前

- [ ] `main` 的 CI 全綠，沒有待處理的 `needs-security-review`。
- [ ] 人工安全檢查（[manual-checks.md](security/manual-checks.md) 第 1、2 節）在要發行的建置上做過，結果寫進該文件最後的紀錄表。0.1.0 之前這個表還是「尚未執行」。
- [ ] 離線驗證的手動檢查（[offline-verification.md](security/offline-verification.md#手動檢查每次發布前)，「每次發布前」）：方法 A（內建的 `scripts/security/watch-connections.ps1`）與方法 B（Process Monitor），在要發行的安裝檔上做；結果依該文件的「紀錄格式」附在發行 PR。
- [ ] 版本：`package.json` 的 `version`、`Cargo.toml` 的 `[workspace.package]`、`Cargo.lock`（`cargo check` 會更新）是同一個版本；`CHANGELOG.md` 有 `## [x.y.z] - yyyy-mm-dd`，日期是發行當天，內容是使用者看得懂的變化。`node scripts/release/check-version.mjs v<版本>` 要通過。
- [ ] 第三方元件聲明是最新的：`node scripts/release/third-party-licenses.mjs --check`。不是的話去掉 `--check` 重新產生並提交（依賴更新之後常會這樣）。
- [ ] 以上的變更走一般的 PR（標題 `chore: release x.y.z`），合併後再繼續。
- [ ] Actions 頁面手動執行 `Release`（選 `main`），全綠。

### 發行

- [ ] 在合併了發行 PR 的 `main` 上打 tag 並推上去：

  ```bash
  git switch main && git pull
  git tag -a v0.1.0 -m "PDF Reader 0.1.0"
  git push origin v0.1.0
  ```

- [ ] 等 `Release` 工作完成，草稿出現在 [Releases](https://github.com/winner0988/Pdf-reader/releases)。
- [ ] 打開草稿：發行說明、兩個附件（`PDF-Reader_<版本>_x64-setup.exe` 與 `.sha256`）都在。
- [ ] 在**乾淨的 Windows 11**（沒有裝過開發工具；虛擬機器即可）下載草稿的安裝檔，PowerShell 的 `Get-FileHash -Algorithm SHA256` 與發行說明表格中的相同；SmartScreen 警告出現時按「其他資訊」→「仍要執行」，安裝（精靈的語言依 Windows 的語言：繁體中文或英文，[packaging.md](architecture/packaging.md#安裝程式的語言rel-06)），開啟一份 PDF，看「關於」的授權、原始碼與第三方元件授權，再解除安裝。
- [ ] 按 **Publish release**（勾選 Set as the latest release）。

### 發行後

- [ ] README 最上面的「狀態：開發中，尚未發布安裝檔」改成連到 Releases 頁面的說明；`SECURITY.md` 的「支援版本」（現在寫「專案尚未發布任何版本，目前只修正 `main`」）改成只修正最新發行的版本（一個一般的 docs PR）。
- [ ] 在 Windows 的舊版上按「檢查更新」，確認會看到新版本；新版上則顯示已是最新。

## 失敗的時候

- `checks` 失敗：訊息說明哪裡不一致。修好之後走一般的 PR 合併，**刪掉 tag 再打在新的 commit 上**：

  ```bash
  git push origin :refs/tags/v0.1.0
  git tag -d v0.1.0
  ```

- `installer` 失敗：與 PR 上的 `Installer (Windows)` 相同的檢查，照該步驟的訊息處理，修好之後同上。
- `draft` 失敗在建立草稿之後（例如附件上傳了一半）：先在 Releases 頁面刪掉那份草稿，再重新執行失敗的工作（Re-run failed jobs）。已經發布的 release 不要刪，改發下一個修訂版。

## 版本與變更記錄

- 版本號依[語意化版本](https://semver.org/lang/zh-TW/)；1.0 之前：新功能升次版號，修正升修訂號，版本之間不保證相容。
- `CHANGELOG.md` 寫給使用者：他們現在能做什麼、哪裡不一樣、已知的限制，用繁體中文；不是 commit 的清單。尚未發行的變更先放在 `## [Unreleased]`（發行 PR 把它改成版本與日期）。
- 發行 PR 只改版本號、變更記錄與（需要時）第三方聲明，不夾帶功能。

## 授權

[ADR 0011](adr/0011-license-and-distribution.md)：AGPL-3.0-or-later；散布時附上 `LICENSE` 與第三方聲明，並說明原始碼在哪裡。

- 安裝檔含 `LICENSE` 與 `THIRD_PARTY_LICENSES.txt`；「關於」寫明授權、原始碼網址與這個版本的 tag，並可以讀第三方元件授權（[packaging.md](architecture/packaging.md#授權與第三方元件聲明rel-04)）。
- 對應的原始碼就是 tag 指到的 commit；Release 頁面上 GitHub 自動附的 Source code 壓縮檔也是它。tag 不可以移動或重新打在別的 commit 上（失敗時刪掉的是**還沒發布**的 tag）。

## 還沒有的

- 程式碼簽章（[DEC-04](backlog/batch-2/DEC-04-installer-code-signing.md)：暫不簽章，附上 SHA-256 並說明 SmartScreen 的警告）。
- 自動更新（[ADR 0009](adr/0009-default-network-policy.md)：從不自動下載或安裝，只告訴使用者有新版本）。
- 其他的發行管道（winget、Microsoft Store）。
