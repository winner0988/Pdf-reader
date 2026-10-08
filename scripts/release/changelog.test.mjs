import assert from "node:assert/strict";
import test from "node:test";

import { section } from "./changelog.mjs";

const CHANGELOG = `# 變更記錄

導言。

## [Unreleased]

- 還沒發行。

## [0.2.0] - 2026-11-01

### 新增

- 第二個版本。

## [0.1.0] - 2026-10-08

第一個版本。

### 閱讀

- 開啟 PDF。
`;

test("a section is its date and the text up to the next section", () => {
  assert.deepEqual(section(CHANGELOG, "0.2.0"), { date: "2026-11-01", body: "### 新增\n\n- 第二個版本。" });
});

test("the last section runs to the end, and a heading inside a section does not end it", () => {
  assert.deepEqual(section(CHANGELOG, "0.1.0"), {
    date: "2026-10-08",
    body: "第一個版本。\n\n### 閱讀\n\n- 開啟 PDF。",
  });
});

test("a version without a dated section has none, however close its number", () => {
  assert.equal(section(CHANGELOG, "0.3.0"), null);
  assert.equal(section(CHANGELOG, "0.1"), null);
  assert.equal(section(CHANGELOG, "0.1.00"), null);
  // "Unreleased" and a heading without a date are not releases.
  assert.equal(section("## [0.4.0]\n\n- x\n", "0.4.0"), null);
  assert.equal(section("## [0.4.0] - soon\n\n- x\n", "0.4.0"), null);
});

test("Windows line endings are read the same", () => {
  assert.deepEqual(section(CHANGELOG.replace(/\n/g, "\r\n"), "0.2.0"), section(CHANGELOG, "0.2.0"));
});
