// #449 的 POC，不合併
//
// Assembles a portable POC folder from a `--features portable` Tauri release build and an
// unpacked Fixed Version WebView2 runtime directory:
//   node scripts/release/build-portable-poc.mjs --webview2 <unpacked-wv2-dir> --out <out-dir> [--target-dir <dir>]
//
// NOTE: This script temporarily creates an empty `src-tauri/WebView2/` placeholder during
// `tauri build` because `tauri-build` checks `bundle.windows.webviewInstallMode.path` during
// compilation even with `--no-bundle`. The placeholder is always removed in `finally`.

import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

function parseArgs(argv) {
  const opts = {
    webview2: process.env.WEBVIEW2_FIXED_RUNTIME_DIR ?? "",
    out: resolve(root, "dist-portable"),
    targetDir: process.env.CARGO_TARGET_DIR || join(root, "target"),
    skipBuild: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--webview2" && i + 1 < argv.length) {
      opts.webview2 = resolve(argv[++i]);
    } else if (arg === "--out" && i + 1 < argv.length) {
      opts.out = resolve(argv[++i]);
    } else if (arg === "--target-dir" && i + 1 < argv.length) {
      opts.targetDir = resolve(argv[++i]);
    } else if (arg === "--skip-build") {
      opts.skipBuild = true;
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  if (!opts.webview2 || !existsSync(join(opts.webview2, "msedgewebview2.exe"))) {
    throw new Error(
      "--webview2 must point to an unpacked Fixed Version WebView2 directory containing msedgewebview2.exe",
    );
  }
  return opts;
}

function runCommand(cmd, args, env) {
  const useShell = process.platform === "win32" && (cmd.endsWith(".cmd") || cmd === "pnpm");
  execFileSync(cmd, args, {
    cwd: root,
    stdio: "inherit",
    shell: useShell,
    env: { ...process.env, ...env },
  });
}

const TARGET = "x86_64-pc-windows-msvc";

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const placeholderWebView2 = join(root, "src-tauri", "WebView2");
  const placeholderCreated = !existsSync(placeholderWebView2);

  try {
    if (!opts.skipBuild) {
      if (placeholderCreated) {
        mkdirSync(placeholderWebView2, { recursive: true });
      }
      runCommand(
        "pnpm",
        [
          "tauri",
          "build",
          "--config",
          "src-tauri/tauri.portable.conf.json",
          "--no-bundle",
          "--target",
          TARGET,
          "--",
          "--features",
          "portable",
        ],
        {
          CARGO_TARGET_DIR: opts.targetDir,
          RUSTFLAGS: `${process.env.RUSTFLAGS ?? ""} -C target-feature=+crt-static`.trim(),
        },
      );
    }
  } finally {
    if (placeholderCreated && existsSync(placeholderWebView2) && readdirSync(placeholderWebView2).length === 0) {
      rmSync(placeholderWebView2, { recursive: true, force: true });
    }
  }

  const releaseDir = join(opts.targetDir, TARGET, "release");
  const appExe = join(releaseDir, "pdf-reader.exe");
  const workerExe = join(releaseDir, "pdf_worker.exe");
  if (!existsSync(appExe) || !existsSync(workerExe)) {
    throw new Error(`Missing release binaries in ${releaseDir} (pdf-reader.exe and pdf_worker.exe required)`);
  }

  rmSync(opts.out, { recursive: true, force: true });
  mkdirSync(join(opts.out, "tessdata"), { recursive: true });

  cpSync(appExe, join(opts.out, "pdf-reader.exe"));
  cpSync(workerExe, join(opts.out, "pdf_worker.exe"));
  cpSync(join(root, "LICENSE"), join(opts.out, "LICENSE"));
  cpSync(join(root, "src/assets/THIRD_PARTY_LICENSES.txt"), join(opts.out, "THIRD_PARTY_LICENSES.txt"));

  const tessdataSrc = join(root, "src-tauri/resources/tessdata");
  for (const name of readdirSync(tessdataSrc)) {
    if (name.endsWith(".traineddata")) {
      cpSync(join(tessdataSrc, name), join(opts.out, "tessdata", name));
    }
  }

  cpSync(opts.webview2, join(opts.out, "WebView2"), { recursive: true });
  console.log(`Portable POC assembled at ${opts.out}`);
}

main();
