import assert from "node:assert/strict";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  checkBrowserArguments,
  checkCapability,
  checkCommandManifest,
  checkCsp,
  checkRepository,
  checkTauriConfig,
  extractDevCsp,
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
const quietWindows = [{ label: "main", additionalBrowserArgs: QUIET_ARGUMENTS }];

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

  const capDir = join(repoRoot, "src-tauri/capabilities");
  for (const file of readdirSync(capDir).filter((name) => name.endsWith(".json"))) {
    copyFileSync(join(capDir, file), join(tempDir, "src-tauri/capabilities", file));
  }

  const buildSource = readFileSync(join(repoRoot, "src-tauri/build.rs"), "utf8");
  const missingCmd = "subscribe_open_events";
  const modifiedBuild = buildSource.replace(`"${missingCmd}",`, "");
  assert.notEqual(buildSource, modifiedBuild, "command should have been removed");
  writeFileSync(join(tempDir, "src-tauri/build.rs"), modifiedBuild, "utf8");

  const problems = checkRepository(tempDir);
  assert.ok(problems.length > 0);
  assert.ok(
    problems.some((p) => p.includes(missingCmd)),
    `expected problem mentioning "${missingCmd}", got: ${JSON.stringify(problems)}`,
  );
});
