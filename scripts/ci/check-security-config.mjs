#!/usr/bin/env node
// Checks security-relevant configuration (AGENTS.md principles 1 and 6, ADR 0009):
//   - the production CSP in src-tauri/tauri.conf.json
//   - the dev-server CSP in vite.config.ts (Tauri does not apply its CSP to devUrl pages)
//   - Tauri capabilities in src-tauri/capabilities/
// Rules only get stricter: loosening one requires the needs-security-review label.
//
// Usage: node scripts/ci/check-security-config.mjs
// Tests: node --test scripts/ci/*.test.mjs

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

/** Directives every policy must set to exactly 'none'. */
const NONE_DIRECTIVES = ["object-src", "base-uri", "form-action", "frame-src"];

/** Directives every policy must define explicitly instead of relying on default-src. */
const EXPLICIT_DIRECTIVES = ["default-src", "script-src", "style-src", "connect-src"];

/** Which directives may use each allowed source. Anything not listed is rejected. */
const PRODUCTION_SOURCES = {
  "'self'": "*",
  "'none'": "*",
  "data:": ["img-src"],
  "blob:": ["img-src", "worker-src"],
  "ipc:": ["connect-src"],
  "http://ipc.localhost": ["connect-src"],
};

/** Extra sources the Vite dev server needs (HMR websocket, React Refresh preamble). */
const DEV_ONLY_SOURCES = {
  "'unsafe-inline'": ["script-src", "style-src"],
  "ws://localhost:1420": ["connect-src"],
};

/**
 * Permissions the frontend may be granted. Add an entry only together with a work card that
 * needs it, a one-line justification, and the needs-security-review label.
 */
const ALLOWED_PERMISSIONS = new Map([
  // The app's own commands (src-tauri/src/commands.rs). They take document and request ids,
  // never paths or URIs; each was reviewed with its card's PR.
  ["allow-subscribe-open-events", "MVP-06: receive open outcomes on a channel instead of core:event"],
  ["allow-open-document-dialog", "MVP-06: the main process shows the dialog; the path stays there"],
  ["allow-retry-open", "MVP-06: reopen the last attempted file after a worker failure"],
  ["allow-close-document", "MVP-06: release a document by its id"],
  ["allow-render-page", "MVP-07: render a page of an open document"],
  ["allow-cancel", "MVP-07, MVP-10: cancel a queued render or a running search by request id"],
  ["allow-get-outline", "MVP-09: read the checked outline"],
  ["allow-search", "MVP-10: full-text search, results on a channel"],
  ["allow-get-page-links", "MVP-12: where a page's links are and where they point"],
  ["allow-describe-link", "MVP-12: what the confirmation dialog shows, checked in the main process"],
  ["allow-open-link", "MVP-12: open a confirmed http/https/mailto link by its id"],
  ["allow-describe-outline-link", "#49: the same confirmation for an outline item"],
  ["allow-open-outline-link", "#49: open a confirmed outline link by its position"],
]);

/** Plugin permission prefixes that are never granted to the frontend, allowlist or not. */
const FORBIDDEN_PERMISSION_PREFIXES = [
  "http:",
  "websocket:",
  "upload:",
  "updater:",
  "shell:",
  "opener:",
  "fs:",
  "process:",
];

/**
 * WebView2 install modes that never touch the network (REL-02). Tauri's default,
 * downloadBootstrapper, and embedBootstrapper fetch the runtime from Microsoft during setup.
 */
const OFFLINE_WEBVIEW2_MODES = ["skip", "offlineInstaller", "fixedRuntime"];

/** Plugins with network or broad system access; they must not be configured at all. */
const FORBIDDEN_PLUGINS = ["http", "websocket", "upload", "updater", "shell", "opener", "fs"];

/** Parses a CSP given as a string ("a b; c d") or as Tauri's object form. */
export function parseCsp(csp) {
  const directives = new Map();
  const entries =
    typeof csp === "string"
      ? csp
          .split(";")
          .map((part) => part.trim().split(/\s+/))
          .filter((tokens) => tokens[0])
          .map(([name, ...sources]) => [name, sources])
      : Object.entries(csp).map(([name, value]) => [
          name,
          (Array.isArray(value) ? value.join(" ") : String(value)).split(/\s+/).filter(Boolean),
        ]);
  for (const [name, sources] of entries) {
    directives.set(name.toLowerCase(), sources);
  }
  return directives;
}

/** Returns a list of problems with a content security policy. */
export function checkCsp(csp, { dev = false, label = "CSP" } = {}) {
  if (csp === undefined || csp === null) {
    return [`${label}: no content security policy is set`];
  }
  const problems = [];
  const directives = parseCsp(csp);

  for (const name of EXPLICIT_DIRECTIVES) {
    if (!directives.has(name)) {
      problems.push(`${label}: missing ${name}`);
    }
  }
  for (const name of NONE_DIRECTIVES) {
    const sources = directives.get(name);
    if (!sources || sources.length !== 1 || sources[0] !== "'none'") {
      problems.push(`${label}: ${name} must be exactly 'none'`);
    }
  }

  const allowed = dev ? { ...PRODUCTION_SOURCES, ...DEV_ONLY_SOURCES } : PRODUCTION_SOURCES;
  for (const [name, sources] of directives) {
    for (const source of sources) {
      const scope = allowed[source.toLowerCase()];
      if (scope === undefined) {
        problems.push(`${label}: ${name} allows ${source}, which is not an allowed source`);
      } else if (scope !== "*" && !scope.includes(name)) {
        problems.push(`${label}: ${source} is not allowed in ${name}`);
      }
    }
  }
  return problems;
}

/** Returns a list of problems with src-tauri/tauri.conf.json. */
export function checkTauriConfig(config) {
  const problems = [];
  const security = config?.app?.security ?? {};

  problems.push(...checkCsp(security.csp, { label: "tauri.conf.json csp" }));
  if ("devCsp" in security) {
    problems.push(
      "tauri.conf.json: remove devCsp; Tauri does not apply it to devUrl pages, the dev policy lives in vite.config.ts",
    );
  }
  if (security.dangerousDisableAssetCspModification) {
    problems.push("tauri.conf.json: dangerousDisableAssetCspModification must not be set");
  }
  if (security.assetProtocol?.enable) {
    problems.push("tauri.conf.json: the asset protocol must stay disabled");
  }
  if (config?.app?.withGlobalTauri) {
    problems.push("tauri.conf.json: withGlobalTauri must stay false");
  }
  for (const plugin of Object.keys(config?.plugins ?? {})) {
    if (FORBIDDEN_PLUGINS.includes(plugin)) {
      problems.push(`tauri.conf.json: plugin "${plugin}" is not allowed`);
    }
  }
  const webviewMode = config?.bundle?.windows?.webviewInstallMode?.type;
  if (!OFFLINE_WEBVIEW2_MODES.includes(webviewMode)) {
    problems.push(
      `tauri.conf.json: bundle.windows.webviewInstallMode must be one of ${OFFLINE_WEBVIEW2_MODES.join(", ")}; ` +
        `"${webviewMode ?? "downloadBootstrapper (the default)"}" makes the installer download WebView2`,
    );
  }
  return problems;
}

/** Extracts the dev CSP string from vite.config.ts, or null when it cannot be found. */
export function extractDevCsp(viteConfigSource) {
  const match = viteConfigSource.match(
    /devContentSecurityPolicy\s*=\s*\[([\s\S]*?)\]\s*\.join\(\s*["']; ["']\s*\)/,
  );
  if (!match) {
    return null;
  }
  // Entries are string literals such as "script-src 'self'" (quotes of the other kind inside).
  const parts = [...match[1].matchAll(/"([^"]*)"|'([^']*)'/g)].map((m) => m[1] ?? m[2]);
  return parts.join("; ");
}

/** Returns a list of problems with one capability file. */
export function checkCapability(capability, fileName) {
  const problems = [];
  if (capability.remote) {
    problems.push(`${fileName}: remote URLs must never get IPC access`);
  }
  for (const entry of capability.permissions ?? []) {
    const id = typeof entry === "string" ? entry : entry?.identifier;
    if (typeof id !== "string") {
      problems.push(`${fileName}: permission entry without an identifier`);
      continue;
    }
    if (FORBIDDEN_PERMISSION_PREFIXES.some((prefix) => id.startsWith(prefix))) {
      problems.push(`${fileName}: ${id} is never granted to the frontend`);
    } else if (!ALLOWED_PERMISSIONS.has(id)) {
      problems.push(
        `${fileName}: ${id} is not in ALLOWED_PERMISSIONS (scripts/ci/check-security-config.mjs)`,
      );
    }
  }
  return problems;
}

/** Runs every check against the repository at `root`. */
export function checkRepository(root) {
  const problems = [];

  const tauriConfig = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
  problems.push(...checkTauriConfig(tauriConfig));

  const devCsp = extractDevCsp(readFileSync(join(root, "vite.config.ts"), "utf8"));
  if (devCsp === null) {
    problems.push("vite.config.ts: devContentSecurityPolicy not found");
  } else {
    problems.push(...checkCsp(devCsp, { dev: true, label: "vite.config.ts dev CSP" }));
  }

  const capabilityDir = join(root, "src-tauri/capabilities");
  for (const file of readdirSync(capabilityDir).filter((name) => name.endsWith(".json"))) {
    const capability = JSON.parse(readFileSync(join(capabilityDir, file), "utf8"));
    problems.push(...checkCapability(capability, `capabilities/${file}`));
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL("../..", import.meta.url));
  const problems = checkRepository(root);
  if (problems.length > 0) {
    for (const problem of problems) {
      console.log(`::error::${problem}`);
    }
    process.exit(1);
  }
  console.log("OK: CSP, dev CSP and capabilities are within policy.");
}
