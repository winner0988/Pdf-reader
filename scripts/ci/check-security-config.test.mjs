import assert from "node:assert/strict";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  ALLOWED_PERMISSIONS_FILE,
  checkAttackSurface,
  checkBrowserArguments,
  checkBundleConfig,
  checkCapability,
  checkCapabilityFolder,
  checkCommandManifest,
  checkCsp,
  checkFuzzingDoc,
  checkPermissionFiles,
  checkRepository,
  checkTauriConfig,
  checkTauriConfigFiles,
  checkWindowsAliases,
  extractDevCsp,
  extractHandlerCommands,
  extractManifestCommands,
  extractRustEnumVariants,
  readAllowedPermissions,
  ungeneratedAppPermissions,
} from "./check-security-config.mjs";

const productionCsp = {
  "default-src": "'self'",
  "script-src": "'self'",
  "style-src": "'self'",
  "img-src": "'self' data: blob:",
  "connect-src": "ipc: http://ipc.localhost",
  "object-src": "'none'",
  "base-uri": "'none'",
  "form-action": "'none'",
  "frame-src": "'none'",
};

const withDirective = (name, value) => ({ ...productionCsp, [name]: value });

test("the repository passes", () => {
  const root = fileURLToPath(new URL("../..", import.meta.url));
  assert.deepEqual(checkRepository(root), []);
});

test("the reference production CSP passes", () => {
  assert.deepEqual(checkCsp(productionCsp), []);
});

test("external hosts are rejected", () => {
  for (const [name, value] of [
    ["connect-src", "ipc: http://ipc.localhost https://beacon.example.invalid"],
    ["font-src", "'self' https://fonts.gstatic.com"],
    ["img-src", "*"],
    ["script-src", "'self' https:"],
  ]) {
    assert.notDeepEqual(checkCsp(withDirective(name, value)), [], `${name} ${value}`);
  }
});

test("unsafe keywords are rejected in production", () => {
  assert.notDeepEqual(checkCsp(withDirective("script-src", "'self' 'unsafe-inline'")), []);
  assert.notDeepEqual(checkCsp(withDirective("script-src", "'self' 'unsafe-eval'")), []);
});

test("sources are only allowed in their own directives", () => {
  assert.notDeepEqual(checkCsp(withDirective("script-src", "'self' data:")), []);
  assert.notDeepEqual(checkCsp(withDirective("img-src", "'self' ipc:")), []);
});

test("hardening directives are required", () => {
  const { "object-src": _, ...withoutObjectSrc } = productionCsp;
  assert.notDeepEqual(checkCsp(withoutObjectSrc), []);
  assert.notDeepEqual(checkCsp(withDirective("frame-src", "'self'")), []);
  const { "connect-src": __, ...withoutConnectSrc } = productionCsp;
  assert.notDeepEqual(checkCsp(withoutConnectSrc), []);
  assert.notDeepEqual(checkCsp(undefined), []);
});

test("string-form policies are parsed", () => {
  const csp = Object.entries(productionCsp)
    .map(([name, value]) => `${name} ${value}`)
    .join("; ");
  assert.deepEqual(checkCsp(csp), []);
});

test("the dev CSP may use the Vite extras, but only where needed", () => {
  const dev = withDirective("connect-src", "'self' ws://localhost:1420 ipc: http://ipc.localhost");
  dev["script-src"] = "'self' 'unsafe-inline'";
  assert.deepEqual(checkCsp(dev, { dev: true }), []);
  assert.notDeepEqual(checkCsp(dev), [], "extras are rejected in production");
  assert.notDeepEqual(
    checkCsp({ ...dev, "connect-src": "'self' ws://evil.example.invalid" }, { dev: true }),
    [],
  );
  assert.notDeepEqual(checkCsp({ ...dev, "img-src": "'self' 'unsafe-inline'" }, { dev: true }), []);
});

test("the dev CSP is extracted from vite.config.ts", () => {
  const source = `const devContentSecurityPolicy = [\n  "default-src 'self'",\n  "object-src 'none'",\n].join("; ");`;
  assert.equal(extractDevCsp(source), "default-src 'self'; object-src 'none'");
  assert.equal(extractDevCsp("export default {}"), null);
});

const offlineBundle = { windows: { webviewInstallMode: { type: "skip" } } };

const QUIET_ARGUMENTS =
  "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-background-networking --no-proxy-server --disable-component-update";
const quietWindows = [{ label: "main", additionalBrowserArgs: QUIET_ARGUMENTS, create: false }];

test("windows must set create: false so setup attaches security handlers", () => {
  const config = (windows) => ({
    app: { security: { csp: productionCsp }, windows },
    bundle: offlineBundle,
  });
  assert.deepEqual(checkTauriConfig(config(quietWindows)), []);
  assert.notDeepEqual(
    checkTauriConfig(config([{ label: "main", additionalBrowserArgs: QUIET_ARGUMENTS, create: true }])),
    [],
  );
  assert.notDeepEqual(
    checkTauriConfig(config([{ label: "main", additionalBrowserArgs: QUIET_ARGUMENTS }])),
    [],
  );
});

test("dangerous Tauri settings are rejected", () => {
  const config = (security, app = {}) => ({
    app: { security: { csp: productionCsp, ...security }, windows: quietWindows, ...app },
    bundle: offlineBundle,
  });
  assert.deepEqual(checkTauriConfig(config({})), []);
  assert.notDeepEqual(checkTauriConfig(config({ devCsp: productionCsp })), []);
  assert.notDeepEqual(checkTauriConfig(config({ dangerousDisableAssetCspModification: true })), []);
  assert.notDeepEqual(checkTauriConfig(config({ assetProtocol: { enable: true } })), []);
  assert.notDeepEqual(checkTauriConfig(config({}, { withGlobalTauri: true })), []);
  assert.notDeepEqual(checkTauriConfig({ ...config({}), plugins: { updater: {} } }), []);
  assert.notDeepEqual(
    checkTauriConfig({ app: { windows: quietWindows }, bundle: offlineBundle }),
    [],
    "a missing CSP is rejected",
  );
});

test("the WebView never connects on its own (#121)", () => {
  assert.deepEqual(checkBrowserArguments(quietWindows), []);
  const window = (additionalBrowserArgs) => [{ label: "main", additionalBrowserArgs }];
  const without = (arg) =>
    QUIET_ARGUMENTS.split(" ")
      .filter((other) => other !== arg)
      .join(" ");
  for (const [args, why] of [
    [undefined, "Tauri's default list: WebView2 fetches its configuration and looks for a proxy"],
    [without("--disable-background-networking"), "background requests"],
    [without("--no-proxy-server"), "proxy discovery"],
    [without("--disable-component-update"), "the component updater"],
    [without("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"), "Tauri's own disabled features are lost"],
    [QUIET_ARGUMENTS.replace(",msSmartScreenProtection", ""), "SmartScreen"],
    [`${QUIET_ARGUMENTS} --remote-debugging-port=9222`, "an open debugging port"],
    [`${QUIET_ARGUMENTS} --proxy-server=http://proxy.example.invalid:8080`, "a proxy of our choosing"],
  ]) {
    assert.notDeepEqual(checkBrowserArguments(window(args)), [], why);
  }
  assert.notDeepEqual(checkBrowserArguments([]), [], "no window");
  assert.notDeepEqual(checkBrowserArguments(undefined), [], "no windows section");
  assert.notDeepEqual(
    checkBrowserArguments([...quietWindows, { label: "other" }]),
    [],
    "every window, not only the first",
  );
});

test("the installer never downloads WebView2", () => {
  const config = (bundle) => ({ app: { security: { csp: productionCsp }, windows: quietWindows }, bundle });
  for (const type of ["skip", "offlineInstaller", "fixedRuntime"]) {
    assert.deepEqual(checkTauriConfig(config({ windows: { webviewInstallMode: { type } } })), [], type);
  }
  for (const type of ["downloadBootstrapper", "embedBootstrapper"]) {
    assert.notDeepEqual(checkTauriConfig(config({ windows: { webviewInstallMode: { type } } })), [], type);
  }
  assert.notDeepEqual(checkTauriConfig(config({ windows: {} })), [], "the default downloads");
  assert.notDeepEqual(checkTauriConfig(config(undefined)), [], "no bundle section downloads too");
});

test("capabilities only grant allowlisted, non-network permissions", () => {
  assert.deepEqual(checkCapability({ permissions: [] }, "main.json"), []);
  assert.notDeepEqual(checkCapability({ permissions: ["http:default"] }, "main.json"), []);
  assert.notDeepEqual(
    checkCapability({ permissions: [{ identifier: "fs:allow-read-file", allow: [{ path: "**" }] }] }, "main.json"),
    [],
  );
  assert.notDeepEqual(checkCapability({ permissions: ["core:path:default"] }, "main.json"), []);
  assert.notDeepEqual(
    checkCapability({ permissions: [], remote: { urls: ["https://example.invalid"] } }, "main.json"),
    [],
  );
});

test("command manifest check passes when commands match", () => {
  const libSource = `
    tauri::Builder::default()
      .invoke_handler(tauri::generate_handler![
        commands::foo,
        commands::bar,
      ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
      "bar",
    ]);
  `;
  assert.deepEqual(checkCommandManifest(libSource, buildSource), []);
});

test("command manifest check fails when manifest is missing a registered command", () => {
  const libSource = `
    .invoke_handler(tauri::generate_handler![
      commands::foo,
      commands::bar,
    ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
    ]);
  `;
  const problems = checkCommandManifest(libSource, buildSource);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /\bbar\b/);
  assert.match(problems[0], /src-tauri\/build\.rs/);
});

test("command manifest check fails when manifest has an unregistered command", () => {
  const libSource = `
    .invoke_handler(tauri::generate_handler![
      commands::foo,
    ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
      "stale_cmd",
    ]);
  `;
  const problems = checkCommandManifest(libSource, buildSource);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /\bstale_cmd\b/);
  assert.match(problems[0], /src-tauri\/src\/lib\.rs/);
});

test("command manifest check ignores comments, newlines, and trailing commas", () => {
  const libSource = `
    .invoke_handler(tauri::generate_handler![
      // commands::commented_out_lib,
      commands::foo, // inline comment
      commands::bar,
    ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      // "commented_out_build",
      "foo", // comment
      "bar",
    ]);
  `;
  assert.deepEqual(checkCommandManifest(libSource, buildSource), []);
});

test("commands:: outside generate_handler! are ignored", () => {
  const libSource = `
    commands::open_paths(app, paths);
    tauri::Builder::default()
      .invoke_handler(tauri::generate_handler![
        commands::foo,
      ])
      .on_window_event(commands::on_window_event);
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
    ]);
  `;
  assert.deepEqual(checkCommandManifest(libSource, buildSource), []);
});

test("command manifest check fails when generate_handler! or AppManifest is missing", () => {
  const validLib = `.invoke_handler(tauri::generate_handler![ commands::foo ])`;
  const validBuild = `AppManifest::new().commands(&["foo"])`;

  const missingLib = checkCommandManifest("fn main() {}", validBuild);
  assert.equal(missingLib.length, 1);
  assert.match(missingLib[0], /generate_handler!/);

  const missingBuild = checkCommandManifest(validLib, "fn main() {}");
  assert.equal(missingBuild.length, 1);
  assert.match(missingBuild[0], /AppManifest/);

  const missingBoth = checkCommandManifest("fn main() {}", "fn main() {}");
  assert.equal(missingBoth.length, 2);
});

test("command manifest check fails when command from another module is missing from manifest", () => {
  const libSource = `
    .invoke_handler(tauri::generate_handler![
      commands::foo,
      other::launch,
    ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
    ]);
  `;
  const problems = checkCommandManifest(libSource, buildSource);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /\blaunch\b/);
  assert.match(problems[0], /src-tauri\/build\.rs/);
});

test("command manifest check fails when bare function command is missing from manifest", () => {
  const libSource = `
    .invoke_handler(tauri::generate_handler![
      commands::foo,
      launch,
    ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
    ]);
  `;
  const problems = checkCommandManifest(libSource, buildSource);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /\blaunch\b/);
  assert.match(problems[0], /src-tauri\/build\.rs/);
});

test("command manifest check passes when module path and bare function commands match manifest", () => {
  const libSource = `
    .invoke_handler(tauri::generate_handler![
      commands::foo,
      other::launch,
      bare_cmd,
    ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
      "launch",
      "bare_cmd",
    ]);
  `;
  assert.deepEqual(checkCommandManifest(libSource, buildSource), []);
});

test("command manifest check reports problem for non-path items in generate_handler!", () => {
  const libSource = `
    .invoke_handler(tauri::generate_handler![
      commands::foo,
      invalid_item(),
    ])
  `;
  const buildSource = `
    let manifest = AppManifest::new().commands(&[
      "foo",
    ]);
  `;
  const problems = checkCommandManifest(libSource, buildSource);
  assert.ok(problems.length >= 1);
  assert.ok(
    problems.some((p) => p.includes("invalid_item()")),
    `expected problem mentioning invalid_item(), got: ${JSON.stringify(problems)}`,
  );
});

test("checkRepository reports problem when manifest is missing a command", (t) => {
  const repoRoot = fileURLToPath(new URL("../..", import.meta.url));
  const tempDir = mkdtempSync(join(tmpdir(), "check-repo-"));
  t.after(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  mkdirSync(join(tempDir, "src-tauri/capabilities"), { recursive: true });
  mkdirSync(join(tempDir, "src-tauri/src"), { recursive: true });
  mkdirSync(join(tempDir, "crates/ipc_contract/src"), { recursive: true });
  mkdirSync(join(tempDir, "docs/security"), { recursive: true });

  copyFileSync(
    join(repoRoot, "src-tauri/tauri.conf.json"),
    join(tempDir, "src-tauri/tauri.conf.json"),
  );
  copyFileSync(
    join(repoRoot, "vite.config.ts"),
    join(tempDir, "vite.config.ts"),
  );
  copyFileSync(
    join(repoRoot, "src-tauri/src/lib.rs"),
    join(tempDir, "src-tauri/src/lib.rs"),
  );
  copyFileSync(
    join(repoRoot, "crates/ipc_contract/src/worker.rs"),
    join(tempDir, "crates/ipc_contract/src/worker.rs"),
  );
  copyFileSync(
    join(repoRoot, "docs/security/attack-surface.md"),
    join(tempDir, "docs/security/attack-surface.md"),
  );
  copyFileSync(
    join(repoRoot, "docs/security/fuzzing.md"),
    join(tempDir, "docs/security/fuzzing.md"),
  );

  const capDir = join(repoRoot, "src-tauri/capabilities");
  for (const file of readdirSync(capDir).filter((name) => name.endsWith(".json"))) {
    copyFileSync(join(capDir, file), join(tempDir, "src-tauri/capabilities", file));
  }
  copyFileSync(
    join(repoRoot, "src-tauri/allowed-permissions.json"),
    join(tempDir, "src-tauri/allowed-permissions.json"),
  );

  const buildSource = readFileSync(join(repoRoot, "src-tauri/build.rs"), "utf8");
  const missingCmd = "subscribe_open_events";
  const modifiedBuild = buildSource.replace(`"${missingCmd}",`, "");
  assert.notEqual(buildSource, modifiedBuild, "command should have been removed");
  writeFileSync(join(tempDir, "src-tauri/build.rs"), modifiedBuild, "utf8");

  const problems = checkRepository(tempDir);
  // The manifest check finds the command; main.json grants, and allowed-permissions.json lists, a
  // permission tauri-build no longer generates (#659, #665).
  assert.equal(problems.length, 3, JSON.stringify(problems));
  assert.ok(problems.every((problem) => problem.includes(missingCmd)), JSON.stringify(problems));
  assert.ok(problems.some((problem) => problem.startsWith("capabilities/main.json: allow-subscribe-open-events ")));
  assert.ok(
    problems.some((problem) =>
      problem.startsWith("src-tauri/allowed-permissions.json: permissions[0]: allow-subscribe-open-events "),
    ),
  );
});

const sampleLib = `
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      commands::cmd_a,
      commands::cmd_b,
      commands::cmd_c,
    ])
`;

const sampleWorker = `
  pub enum WorkerEdit {
    EditOne,
    EditTwo { param: u32 },
  }

  pub enum WorkerRequest {
    ReqOne,
    ReqTwo { id: u32 },
  }
`;

const sampleMainCap = {
  permissions: ["allow-cmd-a", "allow-cmd-b", "allow-cmd-c"],
};

const sampleAttackSurface = `
# 攻擊面清單

## 1. 前端能呼叫的命令（3 個）

### 1.1 測試命令（3 個）

| 命令 | 說明 |
|---|---|
| \`cmd_a\` | 做 A |
| \`cmd_b\` | 做 B |
| \`cmd_c\` | 做 C |

## 2. worker

### 2.4 worker 的請求（2 個）

| 請求 | 說明 |
|---|---|
| \`ReqOne\` | 請求一 |
| \`ReqTwo\` | 請求二 |

### 2.5 編輯（\`WorkerEdit\`，2 種）

| 編輯 | 說明 |
|---|---|
| \`EditOne\` | 編輯一 |
| \`EditTwo\` | 編輯二 |

## 4. WebView 的權限

| 項目 | 設定 |
|---|---|
| capability | 3 個 \`allow-*\` |
`;

test("attack surface check passes when everything matches", () => {
  assert.deepEqual(checkAttackSurface(sampleLib, sampleWorker, sampleMainCap, sampleAttackSurface), []);
});

test("attack surface check fails when a command is missing from attack-surface.md", () => {
  const md = sampleAttackSurface
    .replace("| `cmd_c` | 做 C |\n", "")
    .replaceAll("（3 個）", "（2 個）");
  const problems = checkAttackSurface(sampleLib, sampleWorker, sampleMainCap, md);
  assert.ok(problems.some((p) => p.includes("`cmd_c`（generate_handler!）不在 attack-surface.md §1")));
});

test("attack surface check fails when attack-surface.md has a stale command", () => {
  const md = sampleAttackSurface
    .replace("| `cmd_c` | 做 C |\n", "| `cmd_c` | 做 C |\n| `cmd_stale` | 過時 |\n")
    .replaceAll("（3 個）", "（4 個）");
  const problems = checkAttackSurface(sampleLib, sampleWorker, sampleMainCap, md);
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("`cmd_stale`（attack-surface.md §1）已不存在於 generate_handler!"));
});

test("attack surface check fails when a request is renamed", () => {
  const workerWithRenamed = sampleWorker.replace("ReqTwo", "ReqRenamed");
  const problems = checkAttackSurface(sampleLib, workerWithRenamed, sampleMainCap, sampleAttackSurface);
  assert.equal(problems.length, 2);
  assert.ok(problems.some((p) => p.includes("`ReqRenamed`（WorkerRequest）不在 attack-surface.md §2.4")));
  assert.ok(problems.some((p) => p.includes("`ReqTwo`（attack-surface.md §2.4）已不存在於 WorkerRequest")));
});

test("attack surface check fails when heading quantity does not match table items", () => {
  const mdWrongCount = sampleAttackSurface.replace("### 1.1 測試命令（3 個）", "### 1.1 測試命令（4 個）");
  const problems = checkAttackSurface(sampleLib, sampleWorker, sampleMainCap, mdWrongCount);
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes("標題數量（4）與表格項目數（3）不符"));

  const mdWrongSec1Count = sampleAttackSurface.replace("## 1. 前端能呼叫的命令（3 個）", "## 1. 前端能呼叫的命令（5 個）");
  const problemsSec1 = checkAttackSurface(sampleLib, sampleWorker, sampleMainCap, mdWrongSec1Count);
  assert.equal(problemsSec1.length, 1);
  assert.ok(problemsSec1[0].includes("標題數量（5）與小節項目總數（3）不符"));
});

test("attack surface check fails when capability has a permission without a matching command", () => {
  const capWithOrphan = {
    permissions: [...sampleMainCap.permissions, "allow-orphan-cmd"],
  };
  const problems = checkAttackSurface(sampleLib, sampleWorker, capWithOrphan, sampleAttackSurface);
  assert.equal(problems.length, 1);
  assert.ok(problems[0].includes('權限 "allow-orphan-cmd"'));
  assert.ok(problems[0].includes("`orphan_cmd` 不在 attack-surface.md §1"));
});

test("attack surface check handles multiple names in a single table cell", () => {
  const mdTwoInCell = sampleAttackSurface.replace(
    "| `ReqOne` | 請求一 |\n| `ReqTwo` | 請求二 |",
    "| `ReqOne`、`ReqTwo` | 請求一與二 |",
  );
  assert.deepEqual(checkAttackSurface(sampleLib, sampleWorker, sampleMainCap, mdTwoInCell), []);
});

test("attack surface check verifies non-allow permissions in section 4", () => {
  const capWithPlugin = {
    permissions: [...sampleMainCap.permissions, "core:path:default"],
  };
  const problemsMissing = checkAttackSurface(sampleLib, sampleWorker, capWithPlugin, sampleAttackSurface);
  assert.equal(problemsMissing.length, 1);
  assert.ok(problemsMissing[0].includes('權限 "core:path:default"'));
  assert.ok(problemsMissing[0].includes("不在 attack-surface.md §4"));

  const mdWithPlugin = sampleAttackSurface.replace("3 個 `allow-*`", "3 個 `allow-*` 與 `core:path:default`");
  assert.deepEqual(checkAttackSurface(sampleLib, sampleWorker, capWithPlugin, mdWithPlugin), []);
});

test("extractRustEnumVariants parses Rust enums correctly and handles empty enums", () => {
  const src = `
    #[derive(Debug)]
    pub enum TestEnum {
      /// Comment
      #[serde(default)]
      Alpha,
      Beta { x: u32, y: String },
      Gamma(u32),
      Delta = 42,
    }
    enum EmptyEnum {}
  `;
  assert.deepEqual(extractRustEnumVariants(src, "TestEnum"), ["Alpha", "Beta", "Gamma", "Delta"]);
  assert.deepEqual(extractRustEnumVariants(src, "EmptyEnum"), []);
  assert.equal(extractRustEnumVariants(src, "NonExistent"), null);
});

const sampleFuzzing = `
| 目標 | 測什麼 | 種子 |
|---|---|---|
| \`edit_document\` | 套用編輯（涵蓋 \`WorkerEdit\` 2 種中的 2 種：\`EditOne\`〔含說明〕、\`EditTwo\`；參數經 \`Validate\` 檢查） | 種子 |
`;

test("fuzzing.md counts and lists the WorkerEdit kinds edit_document covers (#674)", () => {
  const problems = (worker, doc) =>
    checkFuzzingDoc(worker, doc).map((problem) => problem.slice(problem.indexOf(": ") + 2));
  assert.deepEqual(problems(sampleWorker, sampleFuzzing), []);
  // A target that covers only some of the kinds lists those.
  assert.deepEqual(problems(sampleWorker, sampleFuzzing.replace("2 種中的 2 種：`EditOne`〔含說明〕、", "2 種中的 1 種：")), []);

  // A new WorkerEdit that the document does not count yet.
  const workerWithThree = sampleWorker.replace("EditTwo { param: u32 },", "EditTwo { param: u32 },\n    EditThree,");
  assert.deepEqual(problems(workerWithThree, sampleFuzzing), [
    "寫 `WorkerEdit` 有 2 種，crates/ipc_contract/src/worker.rs 有 3 種",
  ]);
  // Counted, but left out of the list.
  assert.deepEqual(problems(workerWithThree, sampleFuzzing.replace("2 種中的 2 種", "3 種中的 3 種")), [
    "寫涵蓋 3 種，清單列了 2 種",
  ]);
  // A name that is not a WorkerEdit, and a name listed twice.
  assert.deepEqual(problems(sampleWorker, sampleFuzzing.replace("、`EditTwo`；", "、`EditGone`；")), [
    "清單的 `EditGone` 不是 WorkerEdit 的一種",
  ]);
  assert.deepEqual(problems(sampleWorker, sampleFuzzing.replace("、`EditTwo`；", "、`EditOne`；")), [
    "清單列了兩次 `EditOne`",
  ]);
  // Nothing to compare.
  assert.equal(checkFuzzingDoc(sampleWorker, "# Fuzzing\n").length, 1);
  assert.equal(checkFuzzingDoc("enum Other {}", sampleFuzzing).length, 1);
});

test("checkRepository finds a fuzzing.md that counts the WorkerEdit kinds wrong (#674)", (t) => {
  const repoRoot = fileURLToPath(new URL("../..", import.meta.url));
  const tempDir = mkdtempSync(join(tmpdir(), "check-repo-fuzz-"));
  t.after(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  for (const dir of ["src-tauri/src", "src-tauri/capabilities", "crates/ipc_contract/src", "docs/security"]) {
    mkdirSync(join(tempDir, dir), { recursive: true });
  }
  for (const file of [
    "src-tauri/tauri.conf.json",
    "vite.config.ts",
    "src-tauri/src/lib.rs",
    "src-tauri/build.rs",
    "src-tauri/capabilities/main.json",
    "src-tauri/allowed-permissions.json",
    "crates/ipc_contract/src/worker.rs",
    "docs/security/attack-surface.md",
  ]) {
    copyFileSync(join(repoRoot, file), join(tempDir, file));
  }
  const fuzzing = readFileSync(join(repoRoot, "docs/security/fuzzing.md"), "utf8");
  const stale = fuzzing.replace(/(涵蓋 `WorkerEdit` )(\d+)/, (_, before, total) => `${before}${total - 1}`);
  assert.notEqual(stale, fuzzing, "the count should have been changed");
  writeFileSync(join(tempDir, "docs/security/fuzzing.md"), stale, "utf8");

  const problems = checkRepository(tempDir);
  assert.equal(problems.length, 1, JSON.stringify(problems));
  assert.match(problems[0], /^docs\/security\/fuzzing\.md: 寫 `WorkerEdit` 有 \d+ 種，/);

  rmSync(join(tempDir, "docs/security/fuzzing.md"));
  assert.deepEqual(checkRepository(tempDir), ["docs/security/fuzzing.md: file not found"]);
});

test("checkRepository reports problem when attack surface is missing a command", (t) => {
  const repoRoot = fileURLToPath(new URL("../..", import.meta.url));
  const tempDir = mkdtempSync(join(tmpdir(), "check-repo-atk-"));
  t.after(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });

  mkdirSync(join(tempDir, "src-tauri/capabilities"), { recursive: true });
  mkdirSync(join(tempDir, "src-tauri/src"), { recursive: true });
  mkdirSync(join(tempDir, "crates/ipc_contract/src"), { recursive: true });
  mkdirSync(join(tempDir, "docs/security"), { recursive: true });

  copyFileSync(
    join(repoRoot, "src-tauri/tauri.conf.json"),
    join(tempDir, "src-tauri/tauri.conf.json"),
  );
  copyFileSync(
    join(repoRoot, "vite.config.ts"),
    join(tempDir, "vite.config.ts"),
  );
  copyFileSync(
    join(repoRoot, "src-tauri/src/lib.rs"),
    join(tempDir, "src-tauri/src/lib.rs"),
  );
  copyFileSync(
    join(repoRoot, "src-tauri/build.rs"),
    join(tempDir, "src-tauri/build.rs"),
  );
  copyFileSync(
    join(repoRoot, "crates/ipc_contract/src/worker.rs"),
    join(tempDir, "crates/ipc_contract/src/worker.rs"),
  );

  const capDir = join(repoRoot, "src-tauri/capabilities");
  for (const file of readdirSync(capDir).filter((name) => name.endsWith(".json"))) {
    copyFileSync(join(capDir, file), join(tempDir, "src-tauri/capabilities", file));
  }

  copyFileSync(
    join(repoRoot, "src-tauri/allowed-permissions.json"),
    join(tempDir, "src-tauri/allowed-permissions.json"),
  );

  const docSource = readFileSync(join(repoRoot, "docs/security/attack-surface.md"), "utf8");
  const missingCmd = "subscribe_open_events";
  // Remove line with missingCmd
  const modifiedDoc = docSource.replace(`| \`${missingCmd}\` | 用 channel 接收開檔的結果（不開放 \`core:event\`） | 沒有參數 | — | \`open.spec.ts\` |\n`, "");
  assert.notEqual(docSource, modifiedDoc, "command line should have been removed");
  writeFileSync(join(tempDir, "docs/security/attack-surface.md"), modifiedDoc, "utf8");

  const problems = checkRepository(tempDir);
  assert.ok(problems.length > 0);
  assert.ok(
    problems.some((p) => p.includes(missingCmd)),
    `expected problem mentioning "${missingCmd}", got: ${JSON.stringify(problems)}`,
  );
});

test("AppManifest::commands are read from build.rs", () => {
  const buildSource = `
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
      tauri_build::AppManifest::new().commands(&[
        "cmd_a", // one
        /* "cmd_old", */ r#"cmd_b"#,
      ]),
    ))`;
  assert.deepEqual(extractManifestCommands(buildSource), ["cmd_a", "cmd_b"]);
  assert.equal(extractManifestCommands("fn main() {}"), null);
});

test("app permissions are only the ones tauri-build generates for AppManifest::commands (#659)", () => {
  assert.deepEqual(
    ungeneratedAppPermissions(
      [
        "allow-cmd-a",
        "allow-open-document-dialog",
        "core:default",
        "dialog:allow-open",
        "allow-cmd-x",
        "deny-cmd-a",
        "cmd_a",
        "allow-cmd_a",
      ],
      ["cmd_a", "open_document_dialog"],
    ),
    ["allow-cmd-x", "deny-cmd-a", "cmd_a", "allow-cmd_a"],
  );

  // An allowlisted permission passes while its command is in the manifest, and fails without it.
  const allowed = new Map([
    ["allow-cancel", "test"],
    ["allow-retry-open", "test"],
  ]);
  assert.deepEqual(
    checkCapability({ permissions: ["allow-cancel"] }, "main.json", { allowed, commands: ["cancel"] }),
    [],
  );
  const problems = checkCapability(
    { permissions: ["allow-cancel", { identifier: "allow-retry-open" }] },
    "main.json",
    { allowed, commands: ["retry_open"] },
  );
  assert.equal(problems.length, 1, JSON.stringify(problems));
  assert.match(problems[0], /^main\.json: allow-cancel .*AppManifest::commands .*, and cancel is not there$/);

  // Without the manifest's commands, only the allowlist is checked.
  assert.deepEqual(checkCapability({ permissions: ["allow-cancel"] }, "main.json", { allowed }), []);
  // A forbidden or unlisted permission gets one problem, not a second one about the manifest.
  assert.equal(
    checkCapability({ permissions: ["fs:default", "allow-not-listed"] }, "main.json", { allowed, commands: [] })
      .length,
    2,
  );
});

test("only the permission files tauri-build writes may be in src-tauri/permissions/ (#659)", () => {
  const commands = ["cmd_a", "cmd_b", "cmd_c"];
  // The generated files pass; a command whose file is not written yet is fine.
  assert.deepEqual(checkPermissionFiles(["autogenerated/cmd_a.toml", "autogenerated/cmd_b.toml"], commands), []);
  assert.deepEqual(checkPermissionFiles([], commands), []);

  const extra = [
    "default.toml", // a hand-written permission set
    "sets/core.json", // in another folder, as JSON
    "sets/cmd_b.toml", // named for a command, but tauri-build does not write it there
    "autogenerated/cmd_old.toml", // left behind: tauri-build never deletes a file
    "autogenerated/cmd_a.json", // not the file tauri-build writes
    "autogenerated/Cmd_A.toml",
    "autogenerated/cmd_a.toml/x.toml",
    "schemas/schema.json",
    "autogenerated", // a link where the folder should be
  ];
  const problems = checkPermissionFiles(["autogenerated/cmd_a.toml", ...extra], commands);
  assert.deepEqual(
    problems.map((problem) => problem.slice(0, problem.indexOf(": "))),
    extra.map((file) => `src-tauri/permissions/${file}`).sort(),
  );
  assert.ok(problems.every((problem) => problem.includes("Delete this file")), JSON.stringify(problems));
});

/** Copies the files checkRepository reads into a new temporary folder, removed after the test. */
function copyRepository(t, prefix) {
  const repoRoot = fileURLToPath(new URL("../..", import.meta.url));
  const tempDir = mkdtempSync(join(tmpdir(), prefix));
  t.after(() => {
    rmSync(tempDir, { recursive: true, force: true });
  });
  for (const dir of ["src-tauri/src", "crates/ipc_contract/src", "docs/security"]) {
    mkdirSync(join(tempDir, dir), { recursive: true });
  }
  for (const file of [
    "src-tauri/tauri.conf.json",
    "src-tauri/tauri.bundle.conf.json",
    "src-tauri/allowed-permissions.json",
    "vite.config.ts",
    "src-tauri/src/lib.rs",
    "src-tauri/build.rs",
    "crates/ipc_contract/src/worker.rs",
    "docs/security/attack-surface.md",
    "docs/security/fuzzing.md",
  ]) {
    copyFileSync(join(repoRoot, file), join(tempDir, file));
  }
  for (const dir of ["src-tauri/capabilities", "src-tauri/permissions/autogenerated"]) {
    mkdirSync(join(tempDir, dir), { recursive: true });
    for (const file of readdirSync(join(repoRoot, dir))) {
      copyFileSync(join(repoRoot, dir, file), join(tempDir, dir, file));
    }
  }
  return tempDir;
}

test("checkRepository finds a hand-written permission file (#659)", (t) => {
  const tempDir = copyRepository(t, "check-repo-perm-");
  assert.deepEqual(checkRepository(tempDir), []);

  mkdirSync(join(tempDir, "src-tauri/permissions/sets"));
  writeFileSync(
    join(tempDir, "src-tauri/permissions/sets/open.toml"),
    '[[set]]\nidentifier = "allow-open-plus"\npermissions = ["allow-open-document-dialog", "core:webview:default"]\n',
  );
  writeFileSync(join(tempDir, "src-tauri/permissions/autogenerated/removed_command.toml"), "");
  assert.deepEqual(
    checkRepository(tempDir).map((problem) => problem.slice(0, problem.indexOf(": "))),
    [
      "src-tauri/permissions/autogenerated/removed_command.toml",
      "src-tauri/permissions/sets/open.toml",
    ],
  );
});

test("allowed-permissions.json lists only the app's own allow-<command> permissions, each with a reason (#665)", () => {
  const commands = ["cmd_a", "cmd_b", "cmd_c", "open_document_dialog"];
  const list = (permissions, extra = {}) => JSON.stringify({ $comment: ["test"], permissions, ...extra });

  const good = readAllowedPermissions(
    list([
      { id: "allow-cmd-a", reason: "A" },
      { id: "allow-open-document-dialog", reason: "B" },
    ]),
    commands,
  );
  assert.deepEqual(good.problems, []);
  assert.deepEqual(
    [...good.allowed],
    [
      ["allow-cmd-a", "A"],
      ["allow-open-document-dialog", "B"],
    ],
  );

  const bad = readAllowedPermissions(
    list(
      [
        { id: "core:default", reason: "a core permission" },
        { id: "allow-cmd-x", reason: "no such command" },
        { id: "allow-cmd-b", reason: " " },
        { id: "allow-cmd-a", reason: "A" },
        { id: "allow-cmd-a", reason: "again" },
        { id: "allow-open-document-dialog" },
        { id: "", reason: "no id" },
        "allow-cmd-c",
        { id: "deny-cmd-a", reason: "a deny permission" },
        { id: "allow-cmd_b", reason: "_ instead of -" },
        { id: "allow-cmd-c", reason: "C", scope: ["**"] },
      ],
      { allow: ["core:default"] },
    ),
    commands,
  );
  const file = ALLOWED_PERMISSIONS_FILE;
  const expected = [
    `${file}: unknown key "allow"`,
    `${file}: permissions[0]: core:default has a prefix.`,
    `${file}: permissions[1]: allow-cmd-x has no plugin prefix`,
    `${file}: permissions[2]: allow-cmd-b has no reason`,
    `${file}: permissions[4]: allow-cmd-a is listed twice`,
    `${file}: permissions[5]: allow-open-document-dialog has no reason`,
    `${file}: permissions[6] has no id`,
    `${file}: permissions[7] must be an object with an id and a reason`,
    `${file}: permissions[8]: deny-cmd-a has no plugin prefix`,
    `${file}: permissions[9]: allow-cmd_b has no plugin prefix`,
    `${file}: permissions[10]: allow-cmd-c has keys other than id and reason: scope`,
  ];
  assert.equal(bad.problems.length, expected.length, JSON.stringify(bad.problems, null, 1));
  bad.problems.forEach((problem, index) => assert.ok(problem.startsWith(expected[index]), problem));
  assert.ok(bad.problems[2].endsWith(", and cmd_x is not there"), bad.problems[2]);
  assert.ok(bad.problems[8].endsWith("(src-tauri/build.rs)"), bad.problems[8]);
  assert.ok(bad.problems[9].endsWith("(src-tauri/build.rs)"), bad.problems[9]);
  // A prefixed id never counts as listed, and of two entries for one id the first counts.
  assert.deepEqual(
    [...bad.allowed],
    [
      ["allow-cmd-x", "no such command"],
      ["allow-cmd-b", " "],
      ["allow-cmd-a", "A"],
      ["allow-open-document-dialog", ""],
      ["deny-cmd-a", "a deny permission"],
      ["allow-cmd_b", "_ instead of -"],
      ["allow-cmd-c", "C"],
    ],
  );

  // Without the manifest's commands, the entries are not checked against it.
  assert.deepEqual(readAllowedPermissions(list([{ id: "allow-cmd-x", reason: "r" }]), null).problems, []);
  // Text that is not such a list gives one problem and no list.
  for (const text of ["", "{", "[]", "null", '{"permissions": {}}', '{"$comment": "no list"}']) {
    const { allowed, problems } = readAllowedPermissions(text, commands);
    assert.equal(allowed, null, text);
    assert.equal(problems.length, 1, text);
  }
});

test("capabilities grant only listed app permissions, and core or plugin ones only from the script (#665)", () => {
  const allowed = new Map([["allow-cmd-a", "A"]]);
  assert.deepEqual(checkCapability({ permissions: ["allow-cmd-a"] }, "main.json", { allowed, commands: ["cmd_a"] }), []);
  const problems = checkCapability(
    { permissions: ["allow-cmd-b", "core:event:allow-listen", "dialog:allow-open", "fs:allow-read-file"] },
    "main.json",
    { allowed, commands: ["cmd_a", "cmd_b"] },
  );
  assert.equal(problems.length, 4, JSON.stringify(problems));
  assert.equal(problems[0], `main.json: allow-cmd-b is not in ${ALLOWED_PERMISSIONS_FILE}`);
  assert.ok(
    problems[1].startsWith(
      "main.json: core:event:allow-listen is a core or plugin permission that is not in ALLOWED_PLUGIN_PERMISSIONS",
    ),
    problems[1],
  );
  assert.ok(problems[2].startsWith("main.json: dialog:allow-open is a core or plugin permission"), problems[2]);
  assert.equal(problems[3], "main.json: fs:allow-read-file is never granted to the frontend");

  // When the list could not be read, that is reported once, not for every permission; the
  // permissions are still checked against the manifest.
  assert.deepEqual(
    checkCapability({ permissions: ["allow-cmd-a", "allow-cmd-b"] }, "main.json", {
      allowed: null,
      commands: ["cmd_a", "cmd_b"],
    }),
    [],
  );
  assert.equal(
    checkCapability({ permissions: ["allow-cmd-x"] }, "main.json", { allowed: null, commands: ["cmd_a"] }).length,
    1,
  );
});

test("checkRepository reads the app permissions from src-tauri/allowed-permissions.json (#665)", (t) => {
  const tempDir = copyRepository(t, "check-repo-allowed-");
  const file = join(tempDir, ALLOWED_PERMISSIONS_FILE);
  const list = JSON.parse(readFileSync(file, "utf8"));
  assert.deepEqual(checkRepository(tempDir), []);

  // The list is gone, or is not JSON: one problem, not one for every permission.
  rmSync(file);
  assert.deepEqual(checkRepository(tempDir), [`${ALLOWED_PERMISSIONS_FILE}: file not found`]);
  writeFileSync(file, "{");
  const invalid = checkRepository(tempDir);
  assert.equal(invalid.length, 1, JSON.stringify(invalid));
  assert.ok(invalid[0].startsWith(`${ALLOWED_PERMISSIONS_FILE}: not valid JSON`), invalid[0]);

  // main.json grants a permission the list does not have.
  const index = list.permissions.findIndex((entry) => entry.id === "allow-cancel");
  assert.ok(index >= 0);
  writeFileSync(file, JSON.stringify({ ...list, permissions: list.permissions.filter((_, i) => i !== index) }));
  assert.deepEqual(checkRepository(tempDir), [
    `capabilities/main.json: allow-cancel is not in ${ALLOWED_PERMISSIONS_FILE}`,
  ]);

  // The list has an entry for a command build.rs does not have, and a core permission.
  const count = list.permissions.length;
  writeFileSync(
    file,
    JSON.stringify({
      ...list,
      permissions: [
        ...list.permissions,
        { id: "allow-no-such-command", reason: "test" },
        { id: "core:default", reason: "test" },
      ],
    }),
  );
  const extra = checkRepository(tempDir);
  assert.equal(extra.length, 2, JSON.stringify(extra));
  assert.ok(
    extra[0].startsWith(`${ALLOWED_PERMISSIONS_FILE}: permissions[${count}]: allow-no-such-command `),
    extra[0],
  );
  assert.ok(
    extra[1].startsWith(`${ALLOWED_PERMISSIONS_FILE}: permissions[${count + 1}]: core:default has a prefix`),
    extra[1],
  );
});

/** The part of each problem before ": ", which names the file or setting. */
const subjects = (problems) => problems.map((problem) => problem.slice(0, problem.indexOf(": ")));

/** A capability that grants a core permission, which ALLOWED_PLUGIN_PERMISSIONS does not have. */
const coreCapability = { identifier: "extra", windows: ["main"], permissions: ["core:default"] };

test("src-tauri/capabilities/ holds only .json capability files, at its top (#671)", () => {
  const file = (name) => ({ name, folder: false });
  assert.deepEqual(checkCapabilityFolder([file("main.json"), file("other.json")]), []);
  assert.deepEqual(checkCapabilityFolder([]), []);
  const problems = checkCapabilityFolder([
    file("main.json"),
    file("extra.toml"),
    file("extra.json5"),
    file("MAIN.JSON"),
    file("README.md"),
    { name: "sub", folder: true },
    { name: "schemas", folder: true },
    { name: "folder.json", folder: true },
  ]);
  assert.deepEqual(subjects(problems), [
    "src-tauri/capabilities/MAIN.JSON",
    "src-tauri/capabilities/README.md",
    "src-tauri/capabilities/extra.json5",
    "src-tauri/capabilities/extra.toml",
    "src-tauri/capabilities/folder.json/",
    "src-tauri/capabilities/schemas/",
    "src-tauri/capabilities/sub/",
  ]);
  assert.match(problems[3], /tauri-build also loads \.toml files there/);
  assert.match(problems[6], /also loads the capability files in the folders under/);
});

test("a capability file holds one capability, not a list Tauri would read instead (#671)", () => {
  assert.deepEqual(checkCapability({ identifier: "main", permissions: [] }, "main.json"), []);
  const forms = [
    [coreCapability],
    { capabilities: [coreCapability] },
    // Tauri reads only the list here, so the file's own (allowed) permissions say nothing.
    { identifier: "main", permissions: [], capabilities: [coreCapability] },
    null,
    "main",
  ];
  for (const form of forms) {
    const problems = checkCapability(form, "main.json");
    assert.equal(problems.length, 1, `${JSON.stringify(form)}: ${JSON.stringify(problems)}`);
    assert.match(problems[0], /^main\.json: (must hold one capability|has a "capabilities" list)/);
  }
});

test("tauri.conf.json names capabilities but holds none (#671)", () => {
  const config = (capabilities) => ({
    app: { security: { csp: productionCsp, capabilities }, windows: quietWindows },
    bundle: offlineBundle,
  });
  assert.deepEqual(checkTauriConfig(config(undefined)), []);
  assert.deepEqual(checkTauriConfig(config([])), []);
  assert.deepEqual(checkTauriConfig(config(["main"])), []);
  const inline = checkTauriConfig(config(["main", coreCapability]));
  assert.equal(inline.length, 1, JSON.stringify(inline));
  assert.match(inline[0], /^tauri\.conf\.json: app\.security\.capabilities\[1\] is a capability written into the configuration/);
  assert.deepEqual(checkTauriConfig(config("main")), [
    "tauri.conf.json: app.security.capabilities must be a list of capability identifiers",
  ]);
});

test("tauri.conf.json and tauri.bundle.conf.json are the only Tauri configuration files (#671)", () => {
  const usual = ["build.rs", "capabilities", "Cargo.toml", "tauri.bundle.conf.json", "tauri.conf.json", "tauri-notes.md"];
  assert.deepEqual(checkTauriConfigFiles(usual), []);
  for (const name of [
    "tauri.windows.conf.json",
    "tauri.windows.conf.json5",
    "Tauri.windows.toml",
    "Tauri.Windows.Conf.json",
    "tauri.linux.conf.json",
    "tauri.conf.json5",
    "Tauri.toml",
    "TAURI.CONF.JSON",
    "Tauri.bundle.conf.json",
  ]) {
    const problems = checkTauriConfigFiles([...usual, name]);
    assert.deepEqual(subjects(problems), [`src-tauri/${name}`], JSON.stringify(problems));
  }
  assert.match(checkTauriConfigFiles(["TAURI.CONF.JSON"])[0], /differs from src-tauri\/tauri\.conf\.json only in letter case/);
  assert.match(checkTauriConfigFiles(["tauri.windows.conf.json"])[0], /A Windows build merges tauri\.windows\.conf\.json/);
});

test("tauri.bundle.conf.json sets only what pnpm bundle needs (#671)", () => {
  const repoRoot = fileURLToPath(new URL("../..", import.meta.url));
  const bundle = JSON.parse(readFileSync(join(repoRoot, "src-tauri/tauri.bundle.conf.json"), "utf8"));
  assert.deepEqual(checkBundleConfig(bundle), []);
  assert.deepEqual(checkBundleConfig({}), []);
  for (const [config, setting] of [
    [{ ...bundle, app: { security: { csp: null } } }, "app.security.csp"],
    [{ ...bundle, app: null }, "app"],
    [{ bundle: null }, "bundle"],
    [{ ...bundle, bundle: { ...bundle.bundle, windows: { webviewInstallMode: { type: "downloadBootstrapper" } } } }, "bundle.windows.webviewInstallMode.type"],
    [{ ...bundle, build: { ...bundle.build, devUrl: "http://localhost:1420" } }, "build.devUrl"],
    [{ ...bundle, build: { beforeBuildCommand: { script: "pnpm build" } } }, "build.beforeBuildCommand.script"],
  ]) {
    const problems = checkBundleConfig(config);
    assert.equal(problems.length, 1, JSON.stringify(problems));
    assert.ok(problems[0].startsWith(`src-tauri/tauri.bundle.conf.json: sets ${setting}. `), problems[0]);
  }
  assert.deepEqual(checkBundleConfig([]), ["src-tauri/tauri.bundle.conf.json: must be a JSON object"]);
});

test("names Windows reads as the folders and files the build uses are found (#671)", () => {
  const listings = (dir = null, name = null) => {
    const names = new Map([
      ["", ["src-tauri", "scripts", "vite.config.ts"]],
      ["src-tauri", ["build.rs", "capabilities", "permissions", "src", "tauri.conf.json"]],
      ["src-tauri/src", ["lib.rs", "main.rs"]],
    ]);
    if (dir !== null) {
      names.get(dir).push(name);
    }
    return names;
  };
  assert.deepEqual(checkWindowsAliases(listings()), []);
  assert.deepEqual(checkWindowsAliases(new Map()), []);
  for (const [dir, name, kind] of [
    ["", "SRC-TAURI", "case"],
    ["", "Src-Tauri", "case"],
    ["", "SRC-TA~1", "short"],
    ["", "SCRIPT~1", "short"],
    ["src-tauri", "Capabilities", "case"],
    ["src-tauri", "PERMISSIONS", "case"],
    ["src-tauri", "Build.rs", "case"],
    ["src-tauri", "SRC", "case"],
    ["src-tauri", "CAPABI~1", "short"],
    ["src-tauri", "TAURIC~1.JSO", "short"],
    ["src-tauri/src", "LIB.rs", "case"],
    ["src-tauri/src", "COMMAN~1.RS", "short"],
  ]) {
    const problems = checkWindowsAliases(listings(dir, name));
    assert.deepEqual(subjects(problems), [dir === "" ? name : `${dir}/${name}`], JSON.stringify(problems));
    assert.match(problems[0], kind === "case" ? /only in letter case/ : /short \(8\.3\) name/);
  }
});

test("checkRepository finds what else a Windows build would load (#671)", (t) => {
  const tempDir = copyRepository(t, "check-repo-windows-");
  assert.deepEqual(checkRepository(tempDir), []);
  const capabilities = join(tempDir, "src-tauri/capabilities");

  // Capability files that are not .json files at the top of the folder.
  mkdirSync(join(capabilities, "sub"));
  writeFileSync(join(capabilities, "sub/extra.json"), JSON.stringify(coreCapability));
  writeFileSync(join(capabilities, "extra.toml"), 'identifier = "extra"\nwindows = ["main"]\npermissions = ["core:default"]\n');
  assert.deepEqual(subjects(checkRepository(tempDir)), ["src-tauri/capabilities/extra.toml", "src-tauri/capabilities/sub/"]);
  rmSync(join(capabilities, "sub"), { recursive: true });
  rmSync(join(capabilities, "extra.toml"));

  // A capability file that holds a list.
  writeFileSync(join(capabilities, "extra.json"), JSON.stringify({ capabilities: [coreCapability] }));
  assert.deepEqual(subjects(checkRepository(tempDir)), ["capabilities/extra.json"]);
  rmSync(join(capabilities, "extra.json"));

  // A capability written into tauri.conf.json.
  const configFile = join(tempDir, "src-tauri/tauri.conf.json");
  const configText = readFileSync(configFile, "utf8");
  const config = JSON.parse(configText);
  config.app.security.capabilities = ["main", coreCapability];
  writeFileSync(configFile, JSON.stringify(config));
  assert.deepEqual(subjects(checkRepository(tempDir)), ["tauri.conf.json"]);
  writeFileSync(configFile, configText);

  // A platform configuration file, and a setting tauri.bundle.conf.json may not make.
  writeFileSync(join(tempDir, "src-tauri/tauri.windows.conf.json"), JSON.stringify({ app: { security: { csp: null } } }));
  const bundleFile = join(tempDir, "src-tauri/tauri.bundle.conf.json");
  const bundleText = readFileSync(bundleFile, "utf8");
  writeFileSync(bundleFile, JSON.stringify({ ...JSON.parse(bundleText), app: { security: { csp: null } } }));
  assert.deepEqual(subjects(checkRepository(tempDir)), [
    "src-tauri/tauri.windows.conf.json",
    "src-tauri/tauri.bundle.conf.json",
  ]);
  rmSync(join(tempDir, "src-tauri/tauri.windows.conf.json"));
  writeFileSync(bundleFile, bundleText);
  assert.deepEqual(checkRepository(tempDir), []);

  // A permission set in a folder that differs from src-tauri/permissions/ only in letter case.
  // Without src-tauri/permissions/, this folder can be made on Windows too, where the permission
  // file check also finds the set, since Windows opens src-tauri/permissions/ as this folder.
  rmSync(join(tempDir, "src-tauri/permissions"), { recursive: true });
  mkdirSync(join(tempDir, "src-tauri/Permissions/sets"), { recursive: true });
  writeFileSync(
    join(tempDir, "src-tauri/Permissions/sets/open.toml"),
    '[[set]]\nidentifier = "allow-open-plus"\npermissions = ["allow-open-document-dialog", "core:webview:default"]\n',
  );
  const problems = checkRepository(tempDir);
  assert.equal(subjects(problems)[0], "src-tauri/Permissions", JSON.stringify(problems));
  assert.match(problems[0], /differs from src-tauri\/permissions only in letter case/);
});
