#!/usr/bin/env node
// Checks security-relevant configuration (AGENTS.md principles 1 and 6, ADR 0009):
//   - the production CSP in src-tauri/tauri.conf.json
//   - the dev-server CSP in vite.config.ts (Tauri does not apply its CSP to devUrl pages)
//   - Tauri capabilities in src-tauri/capabilities/, and the list of app permissions they may
//     grant in src-tauri/allowed-permissions.json (#665)
//   - command registration in src-tauri/src/lib.rs vs AppManifest::commands in src-tauri/build.rs (#327)
//   - app permissions: the files in src-tauri/permissions/, and the app permissions capabilities
//     grant, vs AppManifest::commands (#659)
//   - attack surface list in docs/security/attack-surface.md vs code and capabilities (#430)
//   - the WorkerEdit kinds docs/security/fuzzing.md counts and lists for edit_document (#674)
//   - what else a Windows build loads: capability files other than the .json files at the top of
//     src-tauri/capabilities/, capabilities written into a file or the configuration in other
//     forms, Tauri configuration files other than tauri.conf.json, the settings
//     tauri.bundle.conf.json makes, and names Windows treats as the ones this script reads (#671)
// Rules only get stricter: loosening one requires the needs-security-review label.
//
// Usage: node scripts/ci/check-security-config.mjs
// Tests: node --test scripts/ci/*.test.mjs

import { existsSync, readFileSync, readdirSync } from "node:fs";
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
 * The app permissions the frontend may be granted are data, not part of this script (#665): each
 * entry is the allow-<command> tauri-build generates for a command of AppManifest::commands, with
 * a reason. A PR that adds a command adds its permission there, and the guards, which run from the
 * base branch, read the PR's list and check every entry against the PR's build.rs.
 */
export const ALLOWED_PERMISSIONS_FILE = "src-tauri/allowed-permissions.json";

/**
 * Core and plugin permissions (with a prefix, such as core: or dialog:) the frontend may be
 * granted. There are none. Add one only together with a work card that needs it, a one-line
 * justification, and the needs-security-review label. The guards run from the base branch, so
 * the entry must reach main in a PR of its own before the PR that grants the permission.
 */
const ALLOWED_PLUGIN_PERMISSIONS = new Map([]);

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

/**
 * The Tauri configuration files src-tauri/ may hold (#671). A Windows build also merges
 * tauri.windows.conf.json into tauri.conf.json, and with Tauri's config-json5 or config-toml
 * feature it merges tauri.windows.conf.json5 or Tauri.windows.toml, and reads tauri.conf.json5 or
 * Tauri.toml in place of a missing tauri.conf.json. Windows matches those names in any letter case.
 */
const TAURI_CONFIG_FILES = ["tauri.conf.json", "tauri.bundle.conf.json", "tauri.portable.conf.json"];

/**
 * The settings src-tauri/tauri.bundle.conf.json may make, as key paths (#671). `pnpm bundle` runs
 * `tauri build --config src-tauri/tauri.bundle.conf.json`, and Tauri merges that file into
 * tauri.conf.json as a JSON Merge Patch, where null removes a setting (tauri-build and the context
 * macro get it from the CLI in TAURI_CONFIG). Any other setting would change what the installer is
 * built with, unseen by this script.
 */
const BUNDLE_CONFIG_SETTINGS = [["$schema"], ["build", "beforeBuildCommand"], ["bundle", "externalBin"]];

/**
 * The settings src-tauri/tauri.portable.conf.json may make (#449 POC, do not merge as-is).
 */
const PORTABLE_CONFIG_SETTINGS = [
  ["$schema"],
  ["identifier"],
  ["build", "beforeBuildCommand"],
  ["bundle", "externalBin"],
  ["bundle", "windows", "webviewInstallMode", "type"],
  ["bundle", "windows", "webviewInstallMode", "path"],
];

/**
 * Names a Windows build reads where this script reads them, by the folder that holds them (#671).
 * Windows treats names that differ only in letter case as one, so a Windows checkout writes
 * SRC-TAURI/ or src-tauri/Capabilities/ into the folder the build reads, while on the Linux runner
 * they are other folders, which this script never reads. Tauri configuration files are checked
 * by checkTauriConfigFiles.
 */
const WINDOWS_BUILD_NAMES = new Map([
  ["", ["src-tauri"]],
  ["src-tauri", ["capabilities", "permissions", "build.rs", "src"]],
  ["src-tauri/src", ["lib.rs"]],
]);

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

/**
 * WebView2 arguments every window needs (#121). By default WebView2 fetches Microsoft's
 * configuration service and looks for a proxy (WPAD) as the app starts, and its component
 * updater asks Microsoft for updates about a minute later; these stop all three. The list
 * replaces Tauri's own default, so it must keep the features Tauri turns off.
 */
const REQUIRED_BROWSER_ARGUMENTS = ["--disable-background-networking", "--no-proxy-server", "--disable-component-update"];
const REQUIRED_DISABLED_FEATURES = ["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection"];
/** Never in the shipped configuration: an open debugging port, or a proxy of our choosing. */
const FORBIDDEN_BROWSER_ARGUMENTS = ["--remote-debugging-port", "--remote-debugging-pipe", "--proxy-server", "--proxy-pac-url"];

/** Returns a list of problems with the WebView2 arguments of the app's windows. */
export function checkBrowserArguments(windows) {
  const problems = [];
  if (!Array.isArray(windows) || windows.length === 0) {
    return ["tauri.conf.json: app.windows must declare the main window, with its WebView2 arguments"];
  }
  for (const window of windows) {
    const label = `tauri.conf.json: window "${window?.label ?? "?"}" additionalBrowserArgs`;
    const args = String(window?.additionalBrowserArgs ?? "")
      .split(/\s+/)
      .filter(Boolean);
    for (const required of REQUIRED_BROWSER_ARGUMENTS) {
      if (!args.includes(required)) {
        problems.push(`${label} must include ${required}, or WebView2 connects on its own (#121)`);
      }
    }
    const disabled = args
      .filter((arg) => arg.startsWith("--disable-features="))
      .flatMap((arg) => arg.slice("--disable-features=".length).split(","));
    for (const feature of REQUIRED_DISABLED_FEATURES) {
      if (!disabled.includes(feature)) {
        problems.push(`${label} must disable ${feature}, as Tauri's default list (which it replaces) does`);
      }
    }
    for (const forbidden of FORBIDDEN_BROWSER_ARGUMENTS) {
      if (args.some((arg) => arg === forbidden || arg.startsWith(`${forbidden}=`))) {
        problems.push(`${label} must not include ${forbidden}`);
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
  // Tauri also takes capabilities written into the configuration (#671). A list of identifiers
  // only picks among the capability files, which are checked on their own.
  const capabilities = security.capabilities ?? [];
  if (!Array.isArray(capabilities)) {
    problems.push("tauri.conf.json: app.security.capabilities must be a list of capability identifiers");
  } else {
    capabilities.forEach((entry, index) => {
      if (typeof entry !== "string") {
        problems.push(
          `tauri.conf.json: app.security.capabilities[${index}] is a capability written into the ` +
            "configuration. Tauri grants its permissions, but this check reads capabilities only " +
            "from the .json files at the top of src-tauri/capabilities/; move it into one there (#671)",
        );
      }
    });
  }
  for (const window of config?.app?.windows ?? []) {
    if (window?.create !== false) {
      problems.push(
        `tauri.conf.json: window "${window?.label ?? "?"}" must set create: false so setup attaches security handlers`,
      );
    }
  }
  problems.push(...checkBrowserArguments(config?.app?.windows));
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

/** The permission tauri-build generates to allow one command of AppManifest::commands. */
function commandPermission(command) {
  return `allow-${command.replaceAll("_", "-")}`;
}

/**
 * Returns the app permissions among `ids` (those without a plugin prefix) that are not the
 * permission tauri-build generates to allow one of `commands` (#659).
 */
export function ungeneratedAppPermissions(ids, commands) {
  const generated = new Set(commands.map(commandPermission));
  return ids.filter((id) => !id.includes(":") && !generated.has(id));
}

/** Says why an app permission is not one tauri-build generates (#659). */
function notGenerated(id) {
  const command = id.startsWith("allow-") ? id.slice("allow-".length).replaceAll("-", "_") : null;
  return (
    `${id} has no plugin prefix, so it is an app permission, but tauri-build generates one only ` +
    "as allow-<command> for a command in AppManifest::commands (src-tauri/build.rs)" +
    (command !== null && commandPermission(command) === id ? `, and ${command} is not there` : "")
  );
}

/**
 * Reads the app permissions the frontend may be granted from the text of
 * src-tauri/allowed-permissions.json (#665). Returns each listed app permission with its reason
 * (null when the text is not such a list), and the problems with the file: an entry must have an
 * id without a prefix, which is the allow-<command> tauri-build generates for one of `commands`
 * (not checked when null), a reason, and no other keys, and no id may be listed twice.
 */
export function readAllowedPermissions(text, commands, { file = ALLOWED_PERMISSIONS_FILE } = {}) {
  let data;
  try {
    data = JSON.parse(text);
  } catch (error) {
    return { allowed: null, problems: [`${file}: not valid JSON (${error.message})`] };
  }
  if (data === null || !Array.isArray(data.permissions)) {
    return { allowed: null, problems: [`${file}: must be an object with a "permissions" list`] };
  }
  const allowed = new Map();
  const problems = [];
  for (const key of Object.keys(data)) {
    if (key !== "$comment" && key !== "permissions") {
      problems.push(`${file}: unknown key "${key}"`);
    }
  }
  data.permissions.forEach((entry, index) => {
    const where = `${file}: permissions[${index}]`;
    if (entry === null || typeof entry !== "object") {
      problems.push(`${where} must be an object with an id and a reason`);
      return;
    }
    const { id, reason } = entry;
    if (typeof id !== "string" || id === "") {
      problems.push(`${where} has no id`);
      return;
    }
    const extra = Object.keys(entry).filter((key) => key !== "id" && key !== "reason");
    if (extra.length > 0) {
      problems.push(`${where}: ${id} has keys other than id and reason: ${extra.join(", ")}`);
    }
    if (typeof reason !== "string" || reason.trim() === "") {
      problems.push(`${where}: ${id} has no reason`);
    }
    if (id.includes(":")) {
      problems.push(
        `${where}: ${id} has a prefix. Only the app's own allow-<command> permissions are listed ` +
          "here; core and plugin permissions go into ALLOWED_PLUGIN_PERMISSIONS " +
          "(scripts/ci/check-security-config.mjs)",
      );
      return;
    }
    if (allowed.has(id)) {
      problems.push(`${where}: ${id} is listed twice`);
      return;
    }
    if (commands !== null && ungeneratedAppPermissions([id], commands).length > 0) {
      problems.push(`${where}: ${notGenerated(id)}`);
    }
    allowed.set(id, typeof reason === "string" ? reason : "");
  });
  return { allowed, problems };
}

/**
 * Returns a list of problems with one capability file. `allowed` holds the app permissions of
 * src-tauri/allowed-permissions.json (null when that file could not be read; its problem is
 * reported on its own). With `commands` (AppManifest::commands), an app permission must also be
 * the one tauri-build generates for one of them (#659).
 *
 * Tauri also reads a capability file that holds a list of capabilities, or an object with a
 * "capabilities" list, whose own permissions it then ignores (#671). This check reads a file as
 * one capability, so it rejects those forms.
 */
export function checkCapability(capability, fileName, { allowed = new Map(), commands = null } = {}) {
  if (capability === null || typeof capability !== "object" || Array.isArray(capability)) {
    return [
      `${fileName}: must hold one capability, as a JSON object. Tauri loads every capability of a ` +
        "list in one file, but this check reads a file as one capability; keep one capability per file (#671)",
    ];
  }
  if (Object.hasOwn(capability, "capabilities")) {
    return [
      `${fileName}: has a "capabilities" list. Tauri then loads the capabilities in that list and ` +
        "ignores this file's own permissions, but this check reads only the file's own; keep one " +
        "capability per file (#671)",
    ];
  }
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
    } else if (id.includes(":")) {
      if (!ALLOWED_PLUGIN_PERMISSIONS.has(id)) {
        problems.push(
          `${fileName}: ${id} is a core or plugin permission that is not in ALLOWED_PLUGIN_PERMISSIONS ` +
            "(scripts/ci/check-security-config.mjs). The guards run from the base branch, so the " +
            "entry must reach main in a PR of its own first",
        );
      }
    } else if (allowed !== null && !allowed.has(id)) {
      problems.push(`${fileName}: ${id} is not in ${ALLOWED_PERMISSIONS_FILE}`);
    } else if (commands !== null && ungeneratedAppPermissions([id], commands).length > 0) {
      problems.push(`${fileName}: ${notGenerated(id)}`);
    }
  }
  return problems;
}

/**
 * Checks the files in src-tauri/permissions/ (#659). tauri-build loads every .toml and .json file
 * there as an app permission, and a permission set in one can hold core or plugin permissions that
 * checkCapability cannot see behind the set's id. On every build, tauri-build writes
 * autogenerated/<command>.toml for each command of AppManifest::commands, and it never deletes
 * one, so those files are the only ones allowed. `files` are paths relative to `dir`, with /.
 */
export function checkPermissionFiles(files, commands, { dir = "src-tauri/permissions" } = {}) {
  const generated = new Set(commands.map((command) => `autogenerated/${command}.toml`));
  return files
    .filter((file) => !generated.has(file))
    .sort()
    .map(
      (file) =>
        `${dir}/${file}: tauri-build loads every permission file in ${dir}/, so only the ` +
        "autogenerated/<command>.toml it writes for AppManifest::commands (src-tauri/build.rs) may be " +
        "there. Delete this file; a hand-written permission needs its own work card, " +
        "needs-security-review and a change to this check (#659)",
    );
}

/**
 * Lists every entry under `dir` that is not a folder, as paths relative to `dir` with /. Links
 * are listed, not followed. A missing folder has no entries.
 */
function listFiles(dir, prefix = "") {
  if (prefix === "" && !existsSync(dir)) {
    return [];
  }
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      files.push(...listFiles(join(dir, entry.name), `${prefix}${entry.name}/`));
    } else {
      files.push(`${prefix}${entry.name}`);
    }
  }
  return files;
}

/**
 * Checks what the top of src-tauri/capabilities/ holds (#671). tauri-build loads every .json and
 * .toml file in the folder and in the folders under it (and .json5 with the config-json5
 * feature), except files in a folder named schemas, and the configuration does not pick among
 * them, so each capability it loads is granted. This script reads only the .json files at the
 * top, so nothing else may be in the folder. `entries` are { name, folder } for its top.
 */
export function checkCapabilityFolder(entries, { dir = "src-tauri/capabilities" } = {}) {
  return entries
    .filter(({ name, folder }) => folder || !name.endsWith(".json"))
    .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .map(({ name, folder }) =>
      folder
        ? `${dir}/${name}/: tauri-build also loads the capability files in the folders under ${dir}/, ` +
          "but this check reads only the .json files at its top. Keep each capability in a .json " +
          "file there (#671)"
        : `${dir}/${name}: only .json capability files may be in ${dir}/. tauri-build also loads ` +
          ".toml files there (and .json5 with the config-json5 feature), which this check does not read (#671)",
    );
}

/**
 * Checks the names at the top of src-tauri/ that a Windows build may read as Tauri configuration
 * (#671): every name that starts with "tauri.", in any letter case, other than the two files of
 * TAURI_CONFIG_FILES.
 */
export function checkTauriConfigFiles(names, { dir = "src-tauri" } = {}) {
  const problems = [];
  for (const name of [...names].sort()) {
    const lower = name.toLowerCase();
    if (!lower.startsWith("tauri.") || TAURI_CONFIG_FILES.includes(name)) {
      continue;
    }
    problems.push(
      TAURI_CONFIG_FILES.includes(lower)
        ? `${dir}/${name}: differs from ${dir}/${lower} only in letter case. Windows treats the two ` +
            "as one file, so a Windows build may read this one, which this check does not. Rename " +
            "or delete it (#671)"
        : `${dir}/${name}: only ${TAURI_CONFIG_FILES.join(" and ")} may be Tauri configuration ` +
            "files here. A Windows build merges tauri.windows.conf.json, in any letter case, into " +
            "tauri.conf.json (and its .json5 and Tauri.windows.toml forms with Tauri's config-json5 " +
            "and config-toml features), which this check does not see (#671)",
    );
  }
  return problems;
}

/** Lists the key paths of the settings a JSON value makes: the paths to values that are not objects. */
function settingPaths(value, path = []) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return [path];
  }
  return Object.entries(value).flatMap(([key, child]) => settingPaths(child, [...path, key]));
}

function checkOverrideConfig(config, allowedSettings, file, note) {
  if (config === null || typeof config !== "object" || Array.isArray(config)) {
    return [`${file}: must be a JSON object`];
  }
  const allowed = new Set(allowedSettings.map((path) => JSON.stringify(path)));
  return settingPaths(config)
    .filter((path) => !allowed.has(JSON.stringify(path)))
    .map(
      (path) =>
        `${file}: sets ${path.join(".")}. ${note} ` +
        `It may set only ${allowedSettings.map((p) => p.join(".")).join(", ")}. ` +
        "Put other settings in tauri.conf.json, where this check reads them (#671)",
    );
}

/** Returns the problems with src-tauri/tauri.bundle.conf.json: a setting not in BUNDLE_CONFIG_SETTINGS (#671). */
export function checkBundleConfig(config, { file = "src-tauri/tauri.bundle.conf.json" } = {}) {
  return checkOverrideConfig(
    config,
    BUNDLE_CONFIG_SETTINGS,
    file,
    "pnpm bundle merges this file into tauri.conf.json, where a null removes a setting.",
  );
}

/** Returns the problems with src-tauri/tauri.portable.conf.json (#449 POC). */
export function checkPortableConfig(config, { file = "src-tauri/tauri.portable.conf.json" } = {}) {
  const problems = checkOverrideConfig(
    config,
    PORTABLE_CONFIG_SETTINGS,
    file,
    "The portable POC build merges this file into tauri.conf.json, where a null removes a setting.",
  );
  const webviewMode = config?.bundle?.windows?.webviewInstallMode?.type;
  if (webviewMode !== undefined && webviewMode !== "fixedRuntime") {
    problems.push(`${file}: bundle.windows.webviewInstallMode.type must be fixedRuntime, got "${webviewMode}"`);
  }
  return problems;
}

/**
 * Finds names that a Windows checkout writes into a folder or file the build reads (#671): a name
 * in a folder of WINDOWS_BUILD_NAMES that differs from one of its names only in letter case, or
 * one with a ~ and a digit, the form of a short (8.3) name. Windows resolves a short name to the
 * folder or file it belongs to (on a volume that has them, SRC-TA~1 can be src-tauri), so a commit
 * that adds SRC-TA~1/a.json can put a.json into src-tauri/ of an existing checkout. `listings` maps
 * each folder of WINDOWS_BUILD_NAMES to the names in it.
 */
export function checkWindowsAliases(listings) {
  const problems = [];
  for (const [dir, names] of WINDOWS_BUILD_NAMES) {
    const where = (name) => (dir === "" ? name : `${dir}/${name}`);
    for (const name of [...(listings.get(dir) ?? [])].sort()) {
      const same = names.find((known) => known !== name && known.toLowerCase() === name.toLowerCase());
      if (same !== undefined) {
        problems.push(
          `${where(name)}: differs from ${where(same)} only in letter case. Windows treats the two as ` +
            `one, so a Windows checkout puts it where the build reads ${where(same)}, and this check ` +
            "does not read it. Rename or delete it (#671)",
        );
      } else if (/~\d/.test(name)) {
        problems.push(
          `${where(name)}: has the form of a Windows short (8.3) name, which Windows resolves to the ` +
            "folder or file it belongs to (SRC-TA~1 can be src-tauri), so a Windows checkout can write " +
            "it into one the build reads, and this check does not read it. Rename or delete it (#671)",
        );
      }
    }
  }
  return problems;
}

/** Extracts the block inside tauri::generate_handler![ ... ] from lib.rs source. */
export function extractHandlerBlock(libSource) {
  const match = libSource.match(/(?:tauri::)?generate_handler!\s*([\[({])/);
  if (!match) return null;
  const openChar = match[1];
  const closeChar = openChar === "[" ? "]" : openChar === "(" ? ")" : "}";
  const startIdx = match.index + match[0].length;
  let depth = 1;
  let inLineComment = false;
  let inBlockComment = false;
  for (let i = startIdx; i < libSource.length; i++) {
    const ch = libSource[i];
    const next = libSource[i + 1];
    if (inLineComment) {
      if (ch === "\n") inLineComment = false;
      continue;
    }
    if (inBlockComment) {
      if (ch === "*" && next === "/") {
        inBlockComment = false;
        i++;
      }
      continue;
    }
    if (ch === "/" && next === "/") {
      inLineComment = true;
      i++;
      continue;
    }
    if (ch === "/" && next === "*") {
      inBlockComment = true;
      i++;
      continue;
    }
    if (ch === openChar) depth++;
    else if (ch === closeChar) {
      depth--;
      if (depth === 0) {
        return libSource.slice(startIdx, i);
      }
    }
  }
  return null;
}

/** Extracts the block inside AppManifest::new().commands(&[ ... ]) from build.rs source. */
function extractManifestCommandsBlock(buildSource) {
  const match = buildSource.match(
    /(?:tauri_build::)?AppManifest(?:::new\(\))?\s*\.commands\s*\(\s*&?\s*(?:mut\s*)?\[/,
  );
  if (!match) return null;
  const startIdx = match.index + match[0].length;
  let depth = 1;
  let inLineComment = false;
  let inBlockComment = false;
  let inString = false;
  let escape = false;

  for (let i = startIdx; i < buildSource.length; i++) {
    const ch = buildSource[i];
    const next = buildSource[i + 1];

    if (inString) {
      if (escape) {
        escape = false;
      } else if (ch === "\\") {
        escape = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }

    if (inLineComment) {
      if (ch === "\n") inLineComment = false;
      continue;
    }
    if (inBlockComment) {
      if (ch === "*" && next === "/") {
        inBlockComment = false;
        i++;
      }
      continue;
    }

    if (ch === '"') {
      inString = true;
      continue;
    }
    if (ch === "/" && next === "/") {
      inLineComment = true;
      i++;
      continue;
    }
    if (ch === "/" && next === "*") {
      inBlockComment = true;
      i++;
      continue;
    }

    if (ch === "[") depth++;
    else if (ch === "]") {
      depth--;
      if (depth === 0) {
        return buildSource.slice(startIdx, i);
      }
    }
  }
  return null;
}

/** Extracts the command names of AppManifest::commands from build.rs source, or null when missing. */
export function extractManifestCommands(buildSource) {
  const manifestBlock = extractManifestCommandsBlock(buildSource);
  if (manifestBlock === null) {
    return null;
  }
  const cleanedManifest = manifestBlock
    .replace(/\/\/.*$/gm, "")
    .replace(/\/\*[\s\S]*?\*\//g, "");
  return [...cleanedManifest.matchAll(/(?:"([^"]+)"|r#"([^"#]+)"#)/g)].map((m) => m[1] ?? m[2]);
}

/**
 * Checks that every command registered in generate_handler! is declared in AppManifest::commands,
 * and vice-versa (#327, AGENTS.md principle 6).
 */
export function checkCommandManifest(
  libSource,
  buildSource,
  { libPath = "src-tauri/src/lib.rs", buildPath = "src-tauri/build.rs" } = {},
) {
  const problems = [];

  const handlerBlock = extractHandlerBlock(libSource);
  if (handlerBlock === null) {
    problems.push(`${libPath}: tauri::generate_handler! not found`);
  }

  const manifestBlock = extractManifestCommandsBlock(buildSource);
  if (manifestBlock === null) {
    problems.push(`${buildPath}: AppManifest::commands not found`);
  }

  if (handlerBlock === null || manifestBlock === null) {
    return problems;
  }

  const cleanedHandler = handlerBlock
    .replace(/\/\/.*$/gm, "")
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const libCmds = [];
  const RUST_PATH_PATTERN = /^(?:::)?(?:[a-zA-Z_][a-zA-Z0-9_]*)(?:::[a-zA-Z_][a-zA-Z0-9_]*)*$/;
  for (const rawItem of cleanedHandler.split(",")) {
    const item = rawItem.trim();
    if (item === "") {
      continue;
    }
    if (!RUST_PATH_PATTERN.test(item)) {
      problems.push(
        `${libPath}: "${item}" in tauri::generate_handler! is not a valid command path`,
      );
      continue;
    }
    const segments = item.split("::");
    libCmds.push(segments[segments.length - 1]);
  }

  const buildCmds = extractManifestCommands(buildSource);

  const libSet = new Set(libCmds);
  const buildSet = new Set(buildCmds);

  for (const cmd of [...libSet].sort()) {
    if (!buildSet.has(cmd)) {
      problems.push(
        `${buildPath}: command "${cmd}" is registered in ${libPath} but missing from AppManifest::commands (add to ${buildPath})`,
      );
    }
  }

  for (const cmd of [...buildSet].sort()) {
    if (!libSet.has(cmd)) {
      problems.push(
        `${libPath}: command "${cmd}" is listed in ${buildPath} but not registered in tauri::generate_handler! (add to ${libPath} or remove from ${buildPath})`,
      );
    }
  }

  return problems;
}

/** Extracts command names from tauri::generate_handler![ ... ], or null when the macro is missing. */
export function extractHandlerCommands(libSource) {
  const handlerBlock = extractHandlerBlock(libSource);
  if (handlerBlock === null) {
    return null;
  }
  const cleanedHandler = handlerBlock
    .replace(/\/\/.*$/gm, "")
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const libCmds = [];
  const RUST_PATH_PATTERN = /^(?:::)?(?:[a-zA-Z_][a-zA-Z0-9_]*)(?:::[a-zA-Z_][a-zA-Z0-9_]*)*$/;
  for (const rawItem of cleanedHandler.split(",")) {
    const item = rawItem.trim();
    if (item === "") {
      continue;
    }
    if (!RUST_PATH_PATTERN.test(item)) {
      continue;
    }
    const segments = item.split("::");
    libCmds.push(segments[segments.length - 1]);
  }
  return libCmds;
}

/** Extracts variant names of a Rust enum from source code. */
export function extractRustEnumVariants(source, enumName) {
  const enumRegex = new RegExp(`\\b(?:pub\\s+)?enum\\s+${enumName}\\b[^{]*\\{`, "g");
  const match = enumRegex.exec(source);
  if (!match) return null;

  const startIdx = match.index + match[0].length;
  let depth = 1;
  let inLineComment = false;
  let inBlockComment = false;
  let inString = false;
  let escape = false;
  let endIdx = -1;

  for (let i = startIdx; i < source.length; i++) {
    const ch = source[i];
    const next = source[i + 1];

    if (inString) {
      if (escape) escape = false;
      else if (ch === "\\") escape = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (inLineComment) {
      if (ch === "\n") inLineComment = false;
      continue;
    }
    if (inBlockComment) {
      if (ch === "*" && next === "/") {
        inBlockComment = false;
        i++;
      }
      continue;
    }
    if (ch === '"') {
      inString = true;
      continue;
    }
    if (ch === "/" && next === "/") {
      inLineComment = true;
      i++;
      continue;
    }
    if (ch === "/" && next === "*") {
      inBlockComment = true;
      i++;
      continue;
    }
    if (ch === "{") depth++;
    else if (ch === "}") {
      depth--;
      if (depth === 0) {
        endIdx = i;
        break;
      }
    }
  }
  if (endIdx === -1) return null;

  const body = source.slice(startIdx, endIdx);
  const variants = [];
  let braceDepth = 0;
  let parenDepth = 0;
  let bracketDepth = 0;
  inLineComment = false;
  inBlockComment = false;
  inString = false;
  escape = false;

  let expectingVariant = true;

  for (let i = 0; i < body.length; i++) {
    const ch = body[i];
    const next = body[i + 1];

    if (inString) {
      if (escape) escape = false;
      else if (ch === "\\") escape = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (inLineComment) {
      if (ch === "\n") inLineComment = false;
      continue;
    }
    if (inBlockComment) {
      if (ch === "*" && next === "/") {
        inBlockComment = false;
        i++;
      }
      continue;
    }
    if (ch === '"') {
      inString = true;
      continue;
    }
    if (ch === "/" && next === "/") {
      inLineComment = true;
      i++;
      continue;
    }
    if (ch === "/" && next === "*") {
      inBlockComment = true;
      i++;
      continue;
    }

    if (ch === "{") { braceDepth++; continue; }
    if (ch === "}") { if (braceDepth > 0) braceDepth--; continue; }
    if (ch === "(") { parenDepth++; continue; }
    if (ch === ")") { if (parenDepth > 0) parenDepth--; continue; }
    if (ch === "[") { bracketDepth++; continue; }
    if (ch === "]") { if (bracketDepth > 0) bracketDepth--; continue; }

    if (braceDepth === 0 && parenDepth === 0 && bracketDepth === 0) {
      if (ch === ",") {
        expectingVariant = true;
        continue;
      }
      if (ch === "=") {
        expectingVariant = false;
        continue;
      }
      if (expectingVariant && /[a-zA-Z_]/.test(ch)) {
        let ident = "";
        while (i < body.length && /[a-zA-Z0-9_]/.test(body[i])) {
          ident += body[i];
          i++;
        }
        i--;
        variants.push(ident);
        expectingVariant = false;
      }
    }
  }

  return variants;
}

/** Parses markdown into heading sections with content lines. */
function parseMarkdownSections(markdown) {
  const lines = markdown.split(/\r?\n/);
  const sections = [];
  let currentSection = null;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const match = line.match(/^(#{1,6})\s+(.*)$/);
    if (match) {
      const level = match[1].length;
      const title = match[2].trim();
      currentSection = {
        level,
        title,
        line: i + 1,
        contentLines: [],
      };
      sections.push(currentSection);
    } else if (currentSection) {
      currentSection.contentLines.push(line);
    }
  }
  return sections;
}

/** Extracts names enclosed in backticks from the first column of tables in contentLines. */
function extractFirstColumnNames(contentLines) {
  const names = [];
  let headerSeen = false;
  let separatorSeen = false;

  for (const line of contentLines) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("|")) {
      headerSeen = false;
      separatorSeen = false;
      continue;
    }
    if (!headerSeen) {
      headerSeen = true;
      continue;
    }
    if (!separatorSeen) {
      if (/^\|(?:\s*:?-+:?\s*\|)+$/.test(trimmed)) {
        separatorSeen = true;
        continue;
      }
    }
    const cells = trimmed.split("|");
    if (cells.length > 2) {
      const firstColumn = cells[1];
      const backtickMatches = [...firstColumn.matchAll(/`([^`]+)`/g)].map((m) => m[1].trim());
      names.push(...backtickMatches);
    }
  }
  return names;
}

/** Extracts a count number from a heading title, e.g. "（88 個）" or "（`WorkerEdit`，26 種）". */
function extractHeadingCount(title) {
  const match = title.match(/[（(][^）)]*?(\d+)\s*(?:個|種)[）)]/);
  return match ? parseInt(match[1], 10) : null;
}

/**
 * Checks that the attack-surface.md document matches the actual code and configuration (#430):
 *  - commands in generate_handler! match section 1 tables
 *  - WorkerRequest variants match section 2.4 table
 *  - WorkerEdit variants match section 2.5 table
 *  - capabilities/main.json permissions match section 1 and section 4
 *  - heading quantities match table counts
 */
export function checkAttackSurface(
  libSource,
  workerSource,
  mainCapability,
  attackSurfaceSource,
  {
    libPath = "src-tauri/src/lib.rs",
    workerPath = "crates/ipc_contract/src/worker.rs",
    capPath = "src-tauri/capabilities/main.json",
    docPath = "docs/security/attack-surface.md",
  } = {},
) {
  const problems = [];

  const sections = parseMarkdownSections(attackSurfaceSource);

  // 1. Extract commands from generate_handler!
  const codeCmds = extractHandlerCommands(libSource);
  if (codeCmds === null) {
    problems.push(`${libPath}: tauri::generate_handler! not found`);
  }

  // Find Section 1
  const sec1 = sections.find((s) => s.level === 2 && /^1\./.test(s.title));
  if (!sec1) {
    problems.push(`${docPath}: Section 1 ("## 1.") not found`);
  }

  // Find Section 1 subsections (### 1.1, 1.2, ...) between sec1 and sec2
  const sec1Subsections = [];
  let inSec1 = false;
  for (const s of sections) {
    if (s.level === 2) {
      inSec1 = /^1\./.test(s.title);
      continue;
    }
    if (inSec1 && s.level === 3 && /^1\.\d+/.test(s.title)) {
      sec1Subsections.push(s);
    }
  }

  if (sec1 && sec1Subsections.length === 0) {
    problems.push(`${docPath}: Section 1 has no subsections ("### 1.x")`);
  }

  const docCmds = [];
  for (const sub of sec1Subsections) {
    const subCmds = extractFirstColumnNames(sub.contentLines);
    const subCount = extractHeadingCount(sub.title);
    if (subCount === null) {
      problems.push(`${docPath}: "${sub.title}" 標題缺少數量標記（如「（N 個）」）`);
    } else if (subCount !== subCmds.length) {
      problems.push(
        `${docPath}: "${sub.title}" 標題數量（${subCount}）與表格項目數（${subCmds.length}）不符`,
      );
    }
    docCmds.push(...subCmds);
  }

  if (sec1) {
    const sec1Count = extractHeadingCount(sec1.title);
    if (sec1Count === null) {
      problems.push(`${docPath}: "${sec1.title}" 標題缺少數量標記（如「（N 個）」）`);
    } else if (sec1Count !== docCmds.length) {
      problems.push(
        `${docPath}: "${sec1.title}" 標題數量（${sec1Count}）與小節項目總數（${docCmds.length}）不符`,
      );
    }
  }

  // Compare commands bidirectionally
  const docCmdSet = new Set(docCmds);
  if (codeCmds !== null) {
    const codeCmdSet = new Set(codeCmds);
    for (const cmd of codeCmds) {
      if (!docCmdSet.has(cmd)) {
        problems.push(`${docPath}: \`${cmd}\`（generate_handler!）不在 attack-surface.md §1`);
      }
    }
    for (const cmd of docCmds) {
      if (!codeCmdSet.has(cmd)) {
        problems.push(`${docPath}: \`${cmd}\`（attack-surface.md §1）已不存在於 generate_handler!`);
      }
    }
  }

  // 2. WorkerRequest
  const codeRequests = extractRustEnumVariants(workerSource, "WorkerRequest");
  if (codeRequests === null || codeRequests.length === 0) {
    problems.push(`${workerPath}: enum WorkerRequest not found or has no variants`);
  }

  const sec2_4 = sections.find((s) => s.level === 3 && /^2\.4\b/.test(s.title));
  if (!sec2_4) {
    problems.push(`${docPath}: Section 2.4 ("### 2.4") not found`);
  } else {
    const docRequests = extractFirstColumnNames(sec2_4.contentLines);
    const reqCount = extractHeadingCount(sec2_4.title);
    if (reqCount === null) {
      problems.push(`${docPath}: "${sec2_4.title}" 標題缺少數量標記（如「（N 個）」）`);
    } else if (reqCount !== docRequests.length) {
      problems.push(
        `${docPath}: "${sec2_4.title}" 標題數量（${reqCount}）與表格項目數（${docRequests.length}）不符`,
      );
    }

    if (codeRequests !== null && codeRequests.length > 0) {
      const codeReqSet = new Set(codeRequests);
      const docReqSet = new Set(docRequests);
      for (const req of codeRequests) {
        if (!docReqSet.has(req)) {
          problems.push(`${docPath}: \`${req}\`（WorkerRequest）不在 attack-surface.md §2.4`);
        }
      }
      for (const req of docRequests) {
        if (!codeReqSet.has(req)) {
          problems.push(`${docPath}: \`${req}\`（attack-surface.md §2.4）已不存在於 WorkerRequest`);
        }
      }
    }
  }

  // 3. WorkerEdit
  const codeEdits = extractRustEnumVariants(workerSource, "WorkerEdit");
  if (codeEdits === null || codeEdits.length === 0) {
    problems.push(`${workerPath}: enum WorkerEdit not found or has no variants`);
  }

  const sec2_5 = sections.find((s) => s.level === 3 && /^2\.5\b/.test(s.title));
  if (!sec2_5) {
    problems.push(`${docPath}: Section 2.5 ("### 2.5") not found`);
  } else {
    const docEdits = extractFirstColumnNames(sec2_5.contentLines);
    const editCount = extractHeadingCount(sec2_5.title);
    if (editCount === null) {
      problems.push(`${docPath}: "${sec2_5.title}" 標題缺少數量標記（如「（N 種）」）`);
    } else if (editCount !== docEdits.length) {
      problems.push(
        `${docPath}: "${sec2_5.title}" 標題數量（${editCount}）與表格項目數（${docEdits.length}）不符`,
      );
    }

    if (codeEdits !== null && codeEdits.length > 0) {
      const codeEditSet = new Set(codeEdits);
      const docEditSet = new Set(docEdits);
      for (const edit of codeEdits) {
        if (!docEditSet.has(edit)) {
          problems.push(`${docPath}: \`${edit}\`（WorkerEdit）不在 attack-surface.md §2.5`);
        }
      }
      for (const edit of docEdits) {
        if (!codeEditSet.has(edit)) {
          problems.push(`${docPath}: \`${edit}\`（attack-surface.md §2.5）已不存在於 WorkerEdit`);
        }
      }
    }
  }

  // 4. Capabilities (main.json)
  const sec4 = sections.find((s) => s.level === 2 && /^4\./.test(s.title));
  if (!sec4) {
    problems.push(`${docPath}: Section 4 ("## 4.") not found`);
  }
  const sec4Text = sec4 ? sec4.contentLines.join("\n") : "";

  const permissions = mainCapability?.permissions ?? [];
  for (const perm of permissions) {
    const id = typeof perm === "string" ? perm : perm?.identifier;
    if (typeof id !== "string") continue;

    if (id.startsWith("allow-")) {
      const cmd = id.slice(6).replaceAll("-", "_");
      if (!docCmdSet.has(cmd)) {
        problems.push(
          `${docPath}: 權限 "${id}"（${capPath}）對應的命令 \`${cmd}\` 不在 attack-surface.md §1`,
        );
      }
    } else {
      if (!sec4Text.includes(id)) {
        problems.push(`${docPath}: 權限 "${id}"（${capPath}）不在 attack-surface.md §4`);
      }
    }
  }

  return problems;
}

/**
 * Checks the edit_document row of docs/security/fuzzing.md against WorkerEdit (#674). The row
 * says "涵蓋 `WorkerEdit` M 種中的 K 種：`A`、`B`、…；": M must be the number of WorkerEdit
 * variants, and the list must name K different variants. Which ones the fuzz target reaches is
 * not checked here.
 */
export function checkFuzzingDoc(
  workerSource,
  fuzzingSource,
  { workerPath = "crates/ipc_contract/src/worker.rs", docPath = "docs/security/fuzzing.md" } = {},
) {
  const codeEdits = extractRustEnumVariants(workerSource, "WorkerEdit");
  if (codeEdits === null || codeEdits.length === 0) {
    return [`${workerPath}: enum WorkerEdit not found or has no variants`];
  }
  const row = fuzzingSource.match(/涵蓋\s*`WorkerEdit`\s*(\d+)\s*種中的\s*(\d+)\s*種：([^；\n]*)；/);
  if (row === null) {
    return [`${docPath}: edit_document 那一列找不到「涵蓋 \`WorkerEdit\` M 種中的 K 種：…；」`];
  }
  const [, total, covered, list] = row;
  const listed = [...list.matchAll(/`([^`]+)`/g)].map((match) => match[1].trim());
  const problems = [];
  if (Number(total) !== codeEdits.length) {
    problems.push(`${docPath}: 寫 \`WorkerEdit\` 有 ${total} 種，${workerPath} 有 ${codeEdits.length} 種`);
  }
  if (Number(covered) !== listed.length) {
    problems.push(`${docPath}: 寫涵蓋 ${covered} 種，清單列了 ${listed.length} 種`);
  }
  const variants = new Set(codeEdits);
  const seen = new Set();
  for (const name of listed) {
    if (!variants.has(name)) {
      problems.push(`${docPath}: 清單的 \`${name}\` 不是 WorkerEdit 的一種`);
    } else if (seen.has(name)) {
      problems.push(`${docPath}: 清單列了兩次 \`${name}\``);
    }
    seen.add(name);
  }
  return problems;
}

/** Runs every check against the repository at `root`. */
export function checkRepository(root) {
  const problems = [];

  const tauriConfig = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
  problems.push(...checkTauriConfig(tauriConfig));

  // What else a Windows build loads (#671).
  const listings = new Map();
  for (const dir of WINDOWS_BUILD_NAMES.keys()) {
    const path = join(root, dir);
    listings.set(dir, existsSync(path) ? readdirSync(path) : []);
  }
  problems.push(...checkWindowsAliases(listings));
  problems.push(...checkTauriConfigFiles(listings.get("src-tauri")));
  const bundleConfigFile = join(root, "src-tauri/tauri.bundle.conf.json");
  if (existsSync(bundleConfigFile)) {
    problems.push(...checkBundleConfig(JSON.parse(readFileSync(bundleConfigFile, "utf8"))));
  }
  const portableConfigFile = join(root, "src-tauri/tauri.portable.conf.json");
  if (existsSync(portableConfigFile)) {
    problems.push(...checkPortableConfig(JSON.parse(readFileSync(portableConfigFile, "utf8"))));
  }

  const devCsp = extractDevCsp(readFileSync(join(root, "vite.config.ts"), "utf8"));
  if (devCsp === null) {
    problems.push("vite.config.ts: devContentSecurityPolicy not found");
  } else {
    problems.push(...checkCsp(devCsp, { dev: true, label: "vite.config.ts dev CSP" }));
  }

  const libSource = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
  const buildSource = readFileSync(join(root, "src-tauri/build.rs"), "utf8");
  // null when build.rs has no AppManifest::commands; checkCommandManifest reports that.
  const commands = extractManifestCommands(buildSource);

  // null when the list cannot be read: that problem is reported once, not for every permission.
  let allowed = null;
  const allowedFile = join(root, ALLOWED_PERMISSIONS_FILE);
  if (!existsSync(allowedFile)) {
    problems.push(`${ALLOWED_PERMISSIONS_FILE}: file not found`);
  } else {
    const list = readAllowedPermissions(readFileSync(allowedFile, "utf8"), commands);
    problems.push(...list.problems);
    allowed = list.allowed;
  }

  const capabilityDir = join(root, "src-tauri/capabilities");
  const capabilityEntries = readdirSync(capabilityDir, { withFileTypes: true }).map((entry) => ({
    name: entry.name,
    folder: entry.isDirectory(),
  }));
  problems.push(...checkCapabilityFolder(capabilityEntries));
  for (const { name } of capabilityEntries.filter(({ name, folder }) => !folder && name.endsWith(".json"))) {
    const capability = JSON.parse(readFileSync(join(capabilityDir, name), "utf8"));
    problems.push(...checkCapability(capability, `capabilities/${name}`, { allowed, commands }));
  }

  problems.push(...checkCommandManifest(libSource, buildSource));
  if (commands !== null) {
    problems.push(
      ...checkPermissionFiles(listFiles(join(root, "src-tauri/permissions")), commands),
    );
  }

  const workerFile = join(root, "crates/ipc_contract/src/worker.rs");
  const attackSurfaceFile = join(root, "docs/security/attack-surface.md");
  const mainCapFile = join(root, "src-tauri/capabilities/main.json");
  if (!existsSync(workerFile)) {
    problems.push("crates/ipc_contract/src/worker.rs: file not found");
  }
  if (!existsSync(attackSurfaceFile)) {
    problems.push("docs/security/attack-surface.md: file not found");
  }
  if (!existsSync(mainCapFile)) {
    problems.push("src-tauri/capabilities/main.json: file not found");
  }
  if (existsSync(workerFile) && existsSync(attackSurfaceFile) && existsSync(mainCapFile)) {
    const workerSource = readFileSync(workerFile, "utf8");
    const attackSurfaceSource = readFileSync(attackSurfaceFile, "utf8");
    const mainCapability = JSON.parse(readFileSync(mainCapFile, "utf8"));
    problems.push(
      ...checkAttackSurface(libSource, workerSource, mainCapability, attackSurfaceSource),
    );
  }

  const fuzzingFile = join(root, "docs/security/fuzzing.md");
  if (!existsSync(fuzzingFile)) {
    problems.push("docs/security/fuzzing.md: file not found");
  } else if (existsSync(workerFile)) {
    problems.push(
      ...checkFuzzingDoc(readFileSync(workerFile, "utf8"), readFileSync(fuzzingFile, "utf8")),
    );
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
  console.log(
    "OK: CSP, dev CSP, WebView2 arguments, Tauri configuration files, capabilities, allowed app permissions, command manifest, permission files, and attack surface list are within policy.",
  );
}
