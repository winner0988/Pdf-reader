// #598 的 POC，不合併。
// Packages the `pnpm bundle` release binaries and resources into a signed MSIX for #598 POC.
// Reads the signing password from `MSIX_POC_PASSWORD` (never hardcoded).
//
// Usage:
//   node scripts/release/msix-poc/pack.mjs [--out <dir>] [--pfx <file>]

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const POC_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(POC_DIR, "../../..");
const SDK_BIN = "C:\\Program Files (x86)\\Windows Kits\\10\\bin\\10.0.19041.0\\x64";
const MAKEAPPX = path.join(SDK_BIN, "makeappx.exe");
const SIGNTOOL = path.join(SDK_BIN, "signtool.exe");

function parseArgs(argv) {
  let outDir = path.join(tmpdir(), "poc-598");
  let pfxPath = null;
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === "--out" && argv[i + 1]) {
      outDir = path.resolve(argv[++i]);
    } else if (argv[i] === "--pfx" && argv[i + 1]) {
      pfxPath = path.resolve(argv[++i]);
    } else {
      throw new Error(`Unknown argument: ${argv[i]}`);
    }
  }
  pfxPath ??= path.join(outDir, "cert", "pdf-reader-msix-poc.pfx");
  return { outDir, pfxPath };
}

function runTool(exe, args) {
  const result = spawnSync(exe, args, { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${path.basename(exe)} exited with status ${result.status}`);
  }
}

function main() {
  const { outDir, pfxPath } = parseArgs(process.argv.slice(2));
  const password = process.env.MSIX_POC_PASSWORD;
  if (!password) {
    throw new Error("MSIX_POC_PASSWORD environment variable is required to sign the POC MSIX.");
  }

  for (const requiredTool of [MAKEAPPX, SIGNTOOL]) {
    if (!existsSync(requiredTool)) {
      throw new Error(`Missing Windows SDK tool: ${requiredTool}`);
    }
  }
  if (!existsSync(pfxPath)) {
    throw new Error(`Missing PFX certificate: ${pfxPath}`);
  }

  const pkg = JSON.parse(readFileSync(path.join(ROOT, "package.json"), "utf8"));
  const targetDir = process.env.CARGO_TARGET_DIR
    ? path.resolve(process.env.CARGO_TARGET_DIR)
    : path.join(ROOT, "target");

  const mainExe = path.join(targetDir, "release", "pdf-reader.exe");
  const workerExe = path.join(ROOT, "src-tauri", "binaries", "pdf_worker-x86_64-pc-windows-msvc.exe");
  const tessdataDir = path.join(ROOT, "src-tauri", "resources", "tessdata");
  const licenseFile = path.join(ROOT, "LICENSE");
  const noticesFile = path.join(ROOT, "src", "assets", "THIRD_PARTY_LICENSES.txt");
  const manifestFile = path.join(POC_DIR, "AppxManifest.xml");
  const assetsDir = path.join(POC_DIR, "assets");

  for (const requiredFile of [mainExe, workerExe, licenseFile, noticesFile, manifestFile]) {
    if (!existsSync(requiredFile)) {
      throw new Error(`Missing required build/input file: ${requiredFile} (run pnpm bundle first)`);
    }
  }

  const stagingDir = path.join(outDir, "staging");
  rmSync(stagingDir, { recursive: true, force: true });
  mkdirSync(path.join(stagingDir, "tessdata"), { recursive: true });
  mkdirSync(path.join(stagingDir, "Assets"), { recursive: true });

  copyFileSync(manifestFile, path.join(stagingDir, "AppxManifest.xml"));
  copyFileSync(mainExe, path.join(stagingDir, "pdf-reader.exe"));
  copyFileSync(workerExe, path.join(stagingDir, "pdf_worker.exe"));
  copyFileSync(licenseFile, path.join(stagingDir, "LICENSE"));
  copyFileSync(noticesFile, path.join(stagingDir, "THIRD_PARTY_LICENSES.txt"));

  for (const name of readdirSync(tessdataDir)) {
    if (name.endsWith(".traineddata")) {
      copyFileSync(path.join(tessdataDir, name), path.join(stagingDir, "tessdata", name));
    }
  }
  for (const name of readdirSync(assetsDir)) {
    if (name.endsWith(".png")) {
      copyFileSync(path.join(assetsDir, name), path.join(stagingDir, "Assets", name));
    }
  }

  const msixPath = path.join(outDir, `PDF-Reader_${pkg.version}_x64-poc.msix`);
  rmSync(msixPath, { force: true });

  runTool(MAKEAPPX, ["pack", "/o", "/h", "SHA256", "/d", stagingDir, "/p", msixPath]);
  runTool(SIGNTOOL, ["sign", "/fd", "SHA256", "/f", pfxPath, "/p", password, msixPath]);

  const sizeBytes = statSync(msixPath).size;
  const sizeMiB = (sizeBytes / (1024 * 1024)).toFixed(2);
  console.log(`Built and signed: ${msixPath} (${sizeBytes} bytes, ${sizeMiB} MiB)`);
}

main();
