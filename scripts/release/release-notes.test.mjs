import assert from "node:assert/strict";
import test from "node:test";

import { notes, parseChecksum } from "./release-notes.mjs";

const HASH = "0123456789abcdef".repeat(4);
const NAME = "PDF-Reader_0.1.0_x64-setup.exe";
const CHANGELOG = "# 變更記錄\n\n## [0.1.0] - 2026-10-08\n\n第一個版本。\n\n## [0.0.1] - 2026-01-01\n\n舊的。\n";

test("a checksum file is what sha256sum writes", () => {
  assert.deepEqual(parseChecksum(`${HASH}  ${NAME}\n`), { hash: HASH, name: NAME });
  assert.deepEqual(parseChecksum(`${HASH} *${NAME}`), { hash: HASH, name: NAME });
  assert.deepEqual(parseChecksum(`${HASH}  PDF Reader_0.1.0_x64-setup.exe\n`).name, "PDF Reader_0.1.0_x64-setup.exe");
});

test("anything else is refused", () => {
  assert.throws(() => parseChecksum(""), /not '<64 hex digits>/);
  assert.throws(() => parseChecksum(`${HASH.slice(1)}  ${NAME}`), /not '<64 hex digits>/);
  assert.throws(() => parseChecksum(`${HASH.toUpperCase()}  ${NAME}`), /not '<64 hex digits>/);
  assert.throws(() => parseChecksum(HASH), /not '<64 hex digits>/);
});

test("the notes are the section, then the hash, the warning and the source", () => {
  const text = notes({ version: "0.1.0", changelog: CHANGELOG, checksum: `${HASH}  ${NAME}\n` });
  assert.ok(text.startsWith("第一個版本。\n"), "starts with the section of this version");
  assert.ok(!text.includes("舊的"), "not the section of another version");
  assert.ok(text.includes(`| \`${NAME}\` | \`${HASH}\` |`), "the table has the file and its hash");
  assert.ok(text.includes(`Get-FileHash -Algorithm SHA256 -LiteralPath '${NAME}'`), "how to check it");
  assert.ok(text.includes("沒有程式碼簽章"), "says it is not signed");
  assert.ok(text.includes("https://github.com/winner0988/Pdf-reader/tree/v0.1.0"), "links the source of the tag");
});

test("a version without a section has no notes", () => {
  assert.throws(() => notes({ version: "0.2.0", changelog: CHANGELOG, checksum: `${HASH}  ${NAME}` }), /no section for 0\.2\.0/);
});
