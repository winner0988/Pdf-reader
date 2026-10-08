#!/usr/bin/env node
// A release has one version (REL-04, docs/release.md): the one in package.json, in the workspace's
// Cargo.toml and in Cargo.lock for the two programs; the tag, when there is one, is v<version>;
// and the changelog has a dated section for it.
//
//   node scripts/release/check-version.mjs [v<version>]
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { section } from "./changelog.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const PLAIN = /^\d+\.\d+\.\d+$/;
/** The programs the installer carries: their versions in Cargo.lock must be the release's. */
const PROGRAMS = ["pdf-reader", "pdf_worker"];

/** The value of `version = "…"` among the lines `lines`, or null. */
function versionIn(lines) {
  for (const line of lines) {
    const found = line.match(/^version\s*=\s*"([^"]+)"/);
    if (found) return found[1];
  }
  return null;
}

/** `[workspace.package]`'s version. */
export function workspaceVersion(cargoToml) {
  const lines = cargoToml.split("\n");
  const start = lines.findIndex((line) => line.trim() === "[workspace.package]");
  if (start < 0) return null;
  const end = lines.findIndex((line, index) => index > start && line.startsWith("["));
  return versionIn(lines.slice(start + 1, end < 0 ? lines.length : end));
}

/** The version `name` has in Cargo.lock. */
export function lockedVersion(cargoLock, name) {
  for (const block of cargoLock.split("[[package]]")) {
    const lines = block.trim().split("\n");
    if (lines[0] === `name = "${name}"`) return versionIn(lines.slice(1, 2));
  }
  return null;
}

/** What is wrong with a release, as a list of sentences; empty when nothing is. */
export function problems({ packageVersion, workspace, locked, tag, changelog }) {
  const found = [];
  if (!PLAIN.test(packageVersion ?? "")) return [`package.json has no plain x.y.z version (${packageVersion ?? "none"})`];
  if (workspace !== packageVersion) found.push(`Cargo.toml has version ${workspace ?? "none"}, package.json ${packageVersion}`);
  for (const [name, version] of Object.entries(locked)) {
    if (version !== packageVersion) found.push(`Cargo.lock has ${name} ${version ?? "none"}, package.json ${packageVersion}`);
  }
  if (tag && tag !== `v${packageVersion}`) found.push(`the tag is ${tag}, the version is ${packageVersion}: it must be v${packageVersion}`);
  if (!section(changelog, packageVersion)) {
    found.push(`CHANGELOG.md has no section "## [${packageVersion}] - yyyy-mm-dd" for this version`);
  }
  return found;
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? "")) {
  const read = (file) => readFileSync(path.join(ROOT, file), "utf8").replace(/\r\n/g, "\n");
  const lock = read("Cargo.lock");
  const packageVersion = JSON.parse(read("package.json")).version;
  const tag = process.argv[2] || undefined;
  const found = problems({
    packageVersion,
    workspace: workspaceVersion(read("Cargo.toml")),
    locked: Object.fromEntries(PROGRAMS.map((name) => [name, lockedVersion(lock, name)])),
    tag,
    changelog: read("CHANGELOG.md"),
  });
  for (const problem of found) console.log(`::error::${problem}`);
  if (found.length > 0) process.exit(1);
  console.log(`OK: version ${packageVersion}${tag ? `, tag ${tag}` : ""}, in package.json, Cargo.toml, Cargo.lock and CHANGELOG.md.`);
}
