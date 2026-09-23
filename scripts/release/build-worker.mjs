// Builds pdf_worker for the installer (REL-01, docs/architecture/worker-sandbox.md):
// - release profile with a fully static C and C++ runtime, so the worker does not need the
//   VC++ redistributable (MuPDF contains C++ code, which otherwise imports MSVCP140.dll);
// - copied to src-tauri/binaries/pdf_worker-<target>.exe, where bundle.externalBin in
//   src-tauri/tauri.bundle.conf.json expects it. The installer puts it next to the app as
//   pdf_worker.exe.
// Runs as part of `pnpm bundle`; can also be run on its own.

import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { importedDlls, redistributableDlls } from "./pe-imports.mjs";

const TARGET = "x86_64-pc-windows-msvc";
const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

const env = {
  ...process.env,
  // Rust code, and C code built through the cc crate, link the static CRT. An explicit
  // --target keeps these flags away from build scripts, and the output in its own directory.
  RUSTFLAGS: `${process.env.RUSTFLAGS ?? ""} -C target-feature=+crt-static`.trim(),
  // MuPDF's MSBuild projects hard-code /MD; this props file switches them to /MT.
  ForceImportBeforeCppTargets: join(root, "scripts", "release", "static-crt.props"),
};

const cargo = spawnSync(
  "cargo",
  ["build", "--release", "--locked", "-p", "pdf_worker", "--target", TARGET],
  { cwd: root, env, stdio: "inherit" },
);
if (cargo.error) throw cargo.error;
if (cargo.status !== 0) process.exit(cargo.status ?? 1);

const built = join(root, "target", TARGET, "release", "pdf_worker.exe");
const bytes = readFileSync(built);
const redistributable = redistributableDlls(bytes);
if (redistributable.length > 0) {
  console.error(`pdf_worker.exe still needs the VC++ redistributable: ${redistributable.join(", ")}`);
  process.exit(1);
}

const bundled = join(root, "src-tauri", "binaries", `pdf_worker-${TARGET}.exe`);
mkdirSync(dirname(bundled), { recursive: true });
copyFileSync(built, bundled);
const { imports, delayImports } = importedDlls(bytes);
console.log(`pdf_worker.exe -> ${bundled}`);
console.log(`imports: ${[...imports, ...delayImports].join(", ")}`);
