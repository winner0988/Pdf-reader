import assert from "node:assert/strict";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  checkCapability,
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

test("dangerous Tauri settings are rejected", () => {
  const config = (security, app = {}) => ({
    app: { security: { csp: productionCsp, ...security }, ...app },
    bundle: offlineBundle,
  });
  assert.deepEqual(checkTauriConfig(config({})), []);
  assert.notDeepEqual(checkTauriConfig(config({ devCsp: productionCsp })), []);
  assert.notDeepEqual(checkTauriConfig(config({ dangerousDisableAssetCspModification: true })), []);
  assert.notDeepEqual(checkTauriConfig(config({ assetProtocol: { enable: true } })), []);
  assert.notDeepEqual(checkTauriConfig(config({}, { withGlobalTauri: true })), []);
  assert.notDeepEqual(checkTauriConfig({ ...config({}), plugins: { updater: {} } }), []);
  assert.notDeepEqual(checkTauriConfig({ app: {}, bundle: offlineBundle }), [], "a missing CSP is rejected");
});

test("the installer never downloads WebView2", () => {
  const config = (bundle) => ({ app: { security: { csp: productionCsp } }, bundle });
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
