import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { lockedVersion, problems, workspaceVersion } from "./check-version.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const CHANGELOG = "## [0.1.0] - 2026-10-08\n\n- x\n";
const GOOD = {
  packageVersion: "0.1.0",
  workspace: "0.1.0",
  locked: { "pdf-reader": "0.1.0", pdf_worker: "0.1.0" },
  tag: "v0.1.0",
  changelog: CHANGELOG,
};

test("the workspace version is the one of [workspace.package]", () => {
  const toml = `[workspace]
members = ["a"]

[workspace.package]
edition = "2024"
authors = ["someone"]
version = "1.2.3"
license = "AGPL-3.0-or-later"

[workspace.dependencies]
serde = { version = "9.9.9" }

[package]
version = "8.8.8"
`;
  assert.equal(workspaceVersion(toml), "1.2.3");
  assert.equal(workspaceVersion("[package]\nversion = \"1.0.0\"\n"), null);
  // Not another table's version when [workspace.package] has none.
  assert.equal(workspaceVersion("[workspace.package]\nedition = \"2024\"\n\n[package]\nversion = \"8.8.8\"\n"), null);
});

test("a locked version is the one of that exact package", () => {
  const lock = `version = 4

[[package]]
name = "pdf-reader-extra"
version = "9.9.9"

[[package]]
name = "pdf-reader"
version = "0.1.0"
dependencies = [
 "tauri",
]

[[package]]
name = "tauri"
version = "2.11.0"
`;
  assert.equal(lockedVersion(lock, "pdf-reader"), "0.1.0");
  assert.equal(lockedVersion(lock, "tauri"), "2.11.0");
  assert.equal(lockedVersion(lock, "pdf_worker"), null);
});

test("a release that agrees with itself has nothing wrong, with or without a tag", () => {
  assert.deepEqual(problems(GOOD), []);
  assert.deepEqual(problems({ ...GOOD, tag: undefined }), []);
});

test("each disagreement is named", () => {
  assert.match(problems({ ...GOOD, workspace: "0.2.0" }).join("\n"), /Cargo\.toml has version 0\.2\.0, package\.json 0\.1\.0/);
  assert.match(
    problems({ ...GOOD, locked: { ...GOOD.locked, pdf_worker: "0.0.9" } }).join("\n"),
    /Cargo\.lock has pdf_worker 0\.0\.9, package\.json 0\.1\.0/,
  );
  assert.match(problems({ ...GOOD, locked: { ...GOOD.locked, pdf_worker: null } }).join("\n"), /pdf_worker none/);
  assert.match(problems({ ...GOOD, tag: "v0.1.1" }).join("\n"), /the tag is v0\.1\.1.*must be v0\.1\.0/);
  assert.match(problems({ ...GOOD, tag: "0.1.0" }).join("\n"), /must be v0\.1\.0/);
  assert.match(problems({ ...GOOD, changelog: "## [0.2.0] - 2026-11-01\n" }).join("\n"), /no section "## \[0\.1\.0\] - yyyy-mm-dd"/);
});

test("only a plain x.y.z is released", () => {
  for (const packageVersion of ["0.1.0-rc.1", "1.0", "", undefined]) {
    assert.equal(problems({ ...GOOD, packageVersion }).length, 1, String(packageVersion));
  }
});

test("this repository's version, lock file and changelog agree", () => {
  const read = (file) => readFileSync(path.join(ROOT, file), "utf8").replace(/\r\n/g, "\n");
  const lock = read("Cargo.lock");
  assert.deepEqual(
    problems({
      packageVersion: JSON.parse(read("package.json")).version,
      workspace: workspaceVersion(read("Cargo.toml")),
      locked: { "pdf-reader": lockedVersion(lock, "pdf-reader"), pdf_worker: lockedVersion(lock, "pdf_worker") },
      changelog: read("CHANGELOG.md"),
    }),
    [],
  );
});
