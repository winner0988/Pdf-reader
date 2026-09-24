// Usage: node scripts/release/check-imports.mjs <file.exe>...
// Prints each image's DLL imports and fails if any needs the VC++ redistributable (REL-01).

import { readFileSync } from "node:fs";

import { importedDlls, redistributableDlls } from "./pe-imports.mjs";

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("usage: node scripts/release/check-imports.mjs <file.exe>...");
  process.exit(2);
}
let failed = false;
for (const file of files) {
  const bytes = readFileSync(file);
  const { imports, delayImports } = importedDlls(bytes);
  console.log(`${file}: ${[...imports, ...delayImports].join(", ")}`);
  const redistributable = redistributableDlls(bytes);
  if (redistributable.length > 0) {
    console.error(`::error::${file} needs the VC++ redistributable: ${redistributable.join(", ")}`);
    failed = true;
  }
}
process.exit(failed ? 1 : 0);
