#!/usr/bin/env node
// Fails if the built frontend (dist/) loads anything from outside the app:
// external src/href attributes in HTML, or external url()/@import in CSS.
// Everything must be bundled locally (AGENTS.md principle 1).
//
// Usage (after `pnpm build`): node scripts/ci/check-dist.mjs [dist-dir]

import { readFileSync, readdirSync, statSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const EXTERNAL = String.raw`(?:[a-z][a-z0-9+.-]*:)?//`;

const HTML_PATTERNS = [
  new RegExp(String.raw`\b(?:src|href|srcset|action|formaction|poster|data)\s*=\s*["']?\s*${EXTERNAL}`, "gi"),
];

const CSS_PATTERNS = [
  new RegExp(String.raw`url\(\s*["']?\s*${EXTERNAL}`, "gi"),
  new RegExp(String.raw`@import\s+["']\s*${EXTERNAL}`, "gi"),
];

/** Returns the external references found in one file's content. */
export function findExternalReferences(content, kind) {
  const patterns = kind === "html" ? HTML_PATTERNS : CSS_PATTERNS;
  return patterns.flatMap((pattern) => [...content.matchAll(pattern)].map((m) => m[0]));
}

function* walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      yield* walk(path);
    } else {
      yield path;
    }
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL("../..", import.meta.url));
  const dist = process.argv[2] ?? join(root, "dist");
  const problems = [];
  let scanned = 0;
  for (const file of walk(dist)) {
    const kind = { ".html": "html", ".css": "css" }[extname(file)];
    if (!kind) continue;
    scanned++;
    for (const reference of findExternalReferences(readFileSync(file, "utf8"), kind)) {
      problems.push(`${relative(root, file)}: external reference ${reference}`);
    }
  }
  if (scanned === 0) {
    console.log(`::error::no HTML or CSS files found in ${dist}; run pnpm build first`);
    process.exit(1);
  }
  if (problems.length > 0) {
    for (const problem of problems) {
      console.log(`::error::${problem}`);
    }
    process.exit(1);
  }
  console.log(`OK: ${scanned} HTML/CSS files in dist/ load nothing from outside the app.`);
}
