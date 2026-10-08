#!/usr/bin/env node
// The notes of a release (REL-04): its section of CHANGELOG.md, then what everyone who downloads
// the installer needs (DEC-04, ADR 0011): its SHA-256, why Windows warns, where the source is.
//
//   node scripts/release/release-notes.mjs <version> <installer>.sha256 > notes.md
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { section } from "./changelog.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const REPOSITORY = "https://github.com/winner0988/Pdf-reader";

/** `<hash>  <file name>` (the format of sha256sum) as the two. */
export function parseChecksum(text) {
  const match = text.trim().match(/^([0-9a-f]{64}) [ *](.+)$/);
  if (!match) throw new Error("the .sha256 file is not '<64 hex digits>  <file name>'");
  return { hash: match[1], name: match[2] };
}

export function notes({ version, changelog, checksum }) {
  const found = section(changelog, version);
  if (!found) throw new Error(`CHANGELOG.md has no section for ${version}`);
  const { hash, name } = parseChecksum(checksum);
  return `${found.body}

---

### 下載與核對

| 安裝檔 | SHA-256 |
|---|---|
| \`${name}\` | \`${hash}\` |

- 只支援 Windows 11（x64），它內建 WebView2；安裝檔不會下載任何東西。
- **安裝檔沒有程式碼簽章**：執行時 Windows 會顯示「Windows 已保護您的電腦」，發行者是「不明的發行者」。先在 PowerShell 核對雜湊值，與上表相同再按「其他資訊」→「仍要執行」：

  \`\`\`powershell
  Get-FileHash -Algorithm SHA256 -LiteralPath '${name}'
  \`\`\`

### 原始碼與授權

PDF Reader 以 AGPL-3.0-or-later 授權。這個版本的完整原始碼是 [tag v${version}](${REPOSITORY}/tree/v${version})（頁面上的 Source code 壓縮檔也是）；第三方元件的授權在安裝檔與「關於」中。
`;
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? "")) {
  const [version, checksumFile] = process.argv.slice(2);
  if (!version || !checksumFile) {
    console.error("usage: release-notes.mjs <version> <installer>.sha256");
    process.exit(2);
  }
  process.stdout.write(
    notes({
      version,
      changelog: readFileSync(path.join(ROOT, "CHANGELOG.md"), "utf8"),
      checksum: readFileSync(checksumFile, "utf8"),
    }),
  );
}
