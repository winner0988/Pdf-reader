#!/usr/bin/env node
// Checks security-relevant configuration (AGENTS.md principles 1 and 6, ADR 0009):
//   - the production CSP in src-tauri/tauri.conf.json
//   - the dev-server CSP in vite.config.ts (Tauri does not apply its CSP to devUrl pages)
//   - Tauri capabilities in src-tauri/capabilities/
//   - command registration in src-tauri/src/lib.rs vs AppManifest::commands in src-tauri/build.rs (#327)
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
  ["allow-retry-open", "MVP-06, MVP-14: reopen a tab's file after a worker failure, by tab id"],
  ["allow-unlock-tab", "MVP-16, B2-25: try a password on a tab whose file is encrypted; it goes to that tab's worker only, and is remembered only if the user asks"],
  [
    "allow-forget-password",
    "B2-25: forget the password the app remembers for the file of a tab, by tab id; nothing comes back but the tab's new state",
  ],
  [
    "allow-forget-all-passwords",
    "B2-25: forget every password the app remembers in Windows' Credential Manager; answers how many there were",
  ],
  [
    "allow-get-remembered-passwords",
    "B2-25: how many passwords the app remembers, and the most it will; no list, no keys, no passwords",
  ],
  [
    "allow-pick-pages-source",
    "B2-06: a PDF whose pages go into the document; the main process asks in its open dialog and the path stays there, the document's worker scans it and writes a clean copy",
  ],
  [
    "allow-unlock-pages-source",
    "B2-06: the password of the encrypted file just chosen to take pages from; it goes to that document's worker only",
  ],
  ["allow-close-tab", "MVP-14: close a tab by its id; its worker ends"],
  ["allow-set-active-tab", "MVP-14: which tab the window shows, for the window title"],
  ["allow-render-page", "MVP-07: render a page of an open document"],
  ["allow-cancel", "MVP-07, MVP-10: cancel a queued render or a running search by request id"],
  ["allow-get-outline", "MVP-09: read the checked outline"],
  [
    "allow-get-attachments",
    "B3-20, ADR 0025: list the files a document carries (cleaned names, claimed sizes, whether the type is dangerous); by document id, nothing is opened",
  ],
  [
    "allow-save-attachment",
    "B3-20, ADR 0025: save one listed attachment where the user says in the system's save dialog, marked as from the internet; by document and attachment id, never opened",
  ],
  ["allow-search", "MVP-10: full-text search, results on a channel"],
  [
    "allow-search-text-index",
    "B3-21, ADR 0026: search the text index of the documents opened before, results on a channel; each gives a file name, page numbers and a little text, never a path",
  ],
  [
    "allow-open-indexed-document",
    "B3-21, ADR 0026: open the file of a result of the last index search by its number; the path stays in the main process, which checks the file has not changed",
  ],
  ["allow-get-text-index-status", "B3-21, ADR 0026: how many documents the text index has and how much room it takes"],
  ["allow-clear-text-index", "B3-21, ADR 0026: delete the text index, its key first"],
  ["allow-get-page-links", "MVP-12: where a page's links are and where they point"],
  ["allow-get-page-annotations", "B2-07: list one page's annotations (highlights, notes and the document's own) to select and edit them"],
  ["allow-get-page-fields", "B2-09: list one page's form fields (values, options, whether they have scripts) to fill them in; nothing in them is run"],
  ["allow-get-signatures", "B2-14: the signatures of an open document, verified offline by its worker (whether each holds, who signed); by document id"],
  [
    "allow-get-redaction-summary",
    "B2-19, ADR 0016: how many redaction marks, pages, annotations and form fields applying would touch; counts only, by document id",
  ],
  [
    "allow-find-personal-data",
    "#302: scan open document for personal data patterns to mark for redaction; counts and masked previews only, by document id",
  ],
  ["allow-get-page-text", "MVP-15: a page's text and where its characters are, for selecting and copying"],
  ["allow-describe-link", "MVP-12: what the confirmation dialog shows, checked in the main process"],
  ["allow-open-link", "MVP-12: open a confirmed http/https/mailto link by its id"],
  ["allow-describe-outline-link", "#49: the same confirmation for an outline item"],
  ["allow-open-outline-link", "#49: open a confirmed outline link by its position"],
  ["allow-describe-remote-address", "B3-08: the same confirmation for a remote reference's web address"],
  ["allow-open-remote-address", "B3-08: hand a confirmed remote address to the browser by its position"],
  ["allow-open-default-apps-settings", "REL-03: open the fixed default-apps page of Windows Settings"],
  [
    "allow-check-for-updates",
    "#64, ADR 0009: the one network request, from the main process to a fixed GitHub address; no arguments",
  ],
  ["allow-describe-releases-page", "#64: what the link confirmation shows about the fixed releases page"],
  ["allow-open-releases-page", "#64: open the fixed releases page after the user confirmed it"],
  ["allow-get-recent-files", "#73: the recently opened files as names and ids; the paths stay in the main process"],
  ["allow-open-recent-file", "#73: open a recently opened file by its id"],
  ["allow-remove-recent-file", "#73: take a file off the recent files list by its id"],
  ["allow-clear-recent-files", "#73: empty the recent files list"],
  ["allow-get-file-recording", "#73: whether an open document's file may be on the recent files list"],
  ["allow-set-file-recording", "#73: record an open document's file on the recent files list, or not"],
  ["allow-get-settings", "B2-12: the user's settings (theme, whether to record recent files)"],
  ["allow-set-settings", "B2-12: replace the user's settings; typed, no other field accepted"],
  ["allow-clear-recent-exclusions", "B2-12: forget which files the user asked not to record"],
  ["allow-check-sensitive-mark", "B3-05, ADR 0010: what writing the sensitive mark into an open document's file would do"],
  [
    "allow-set-sensitive",
    "B3-05, ADR 0010: append the sensitive mark to an open document's file, or take it out, after the user confirmed; no path from the page",
  ],
  ["allow-get-sensitive-files", "B3-05: the files marked as sensitive on this computer, as names and ids; no paths"],
  ["allow-remove-sensitive-file", "B3-05: take a file off the list of marked files by its id; the file is not changed"],
  [
    "allow-copy-protected-text",
    "B3-05, ADR 0004: put text copied from a sensitive document on the clipboard, kept out of the clipboard history; never reads it",
  ],
  ["allow-export-pages", "B2-04: export pages as text or PNG; the main process asks where and writes the files"],
  [
    "allow-export-annotation-summary",
    "B3-18: a summary of an open document's annotations as a text or Markdown file, by document id; the main process makes it, asks where, never the document's own file; refused when the author forbids copying",
  ],
  [
    "allow-compare-text",
    "B3-24, E9: compare text between two open documents by their document ids; runs on blocking pool; refused when either author forbids copying",
  ],
  [
    "allow-export-compare-text",
    "B3-24, E9: export text comparison as text or Markdown file; main process asks where, never either document's own file",
  ],
  ["allow-apply-edit", "B2-02: a typed edit of an open document, applied in its worker; validated, no PDF objects"],
  ["allow-undo-edit", "B2-05: undo the last edit of an open document; its worker opens the kept bytes again"],
  ["allow-redo-edit", "B2-05: make the last undone edit of an open document again"],
  ["allow-recover-edits", "B2-13: make again the edits an earlier run left for an open document's file (crash recovery)"],
  ["allow-discard-recovered-edits", "B2-13: delete the recovery journal an earlier run left for an open document's file"],
  ["allow-save-document", "B2-02: write an open document to its own file; the main process owns the path"],
  ["allow-save-document-as", "B2-02: write an open document to a file the user picks in the main process's save dialog"],
  ["allow-close-window", "B2-02: close the window after the user was asked about unsaved changes"],
  [
    "allow-privacy-export",
    "B2-03: a copy without metadata; the main process asks where, never the document's own file",
  ],
  ["allow-get-ocr-languages", "B2-10: the languages scanned pages can be recognised in (codes and sizes)"],
  ["allow-get-fonts", "B2-27: the fonts installed in Windows (ids and names, never a path), for text boxes"],
  ["allow-get-text-box", "B2-29: what a text box says of itself (words, font name, style), to fill in the dialog that changes it"],
  ["allow-prepare-text-box", "B2-29: the words of a text box, made into a small subset of a font by id (the page gets no path and no font bytes)"],
  [
    "allow-import-font",
    "B2-32: the main process asks for a .ttf/.ttc file, checks its header and size, the worker checks its TrueType outlines, and copies it into the app's data folder; the path stays there",
  ],
  ["allow-remove-font", "B2-32: remove an imported font by its id; the installed Windows fonts stay"],
  ["allow-remove-all-fonts", "B2-32: remove all imported fonts from the app's data folder; the installed Windows fonts stay"],
  [
    "allow-import-ocr-language",
    "B2-10: the main process asks for a .traineddata file, checks its name, size and format, and copies it into the app's data folder; the path stays there",
  ],
  ["allow-remove-ocr-language", "B2-10: remove an imported language by its code; the ones that came with the app stay"],
  ["allow-start-ocr", "B2-10: recognise the text of an open document's scanned pages now, by document id"],
  ["allow-stop-ocr", "B2-10: stop recognising an open document's pages, by document id"],
  ["allow-set-ocr-focus", "B2-10: the page the user looks at, which is recognised first, by document id and page number"],
  [
    "allow-encrypt-copy",
    "B2-15: a copy encrypted with AES-256; the passwords are used once and not kept, the main process asks where, never the document's own file",
  ],
  [
    "allow-compress-images",
    "B3-01: a copy whose pictures are encoded again, smaller; the main process asks where, never the document's own file, and the worker decodes the pictures as it does for any page",
  ],
  [
    "allow-page-marks-copy",
    "B3-16: a copy with page numbers, Bates numbers, headers and footers drawn on its pages; the main process asks where, never the document's own file",
  ],
  [
    "allow-sanitize-copy",
    "B3-15: a copy without active content, attachments and hidden data, checked by the active-content scan before it is written; the main process asks where, never the document's own file",
  ],
  [
    "allow-run-batch",
    "B3-04: one operation (metadata removal, encrypted copy) done to many files the user picks, one after the other, each in a worker of its own; the main process asks for the files and the folder, the paths stay there, no file is changed",
  ],
  [
    "allow-pick-stamp-image",
    "B2-08: a picture for a custom stamp; the main process asks in its open dialog and the path stays there, the document's worker keeps only the pixels",
  ],
  [
    "allow-create-pdf",
    "B3-03: a PDF made of files the user picks; the main process asks for the files and for where the PDF goes, a sandboxed worker reads them through read-only handles, and the paths stay there",
  ],
  ["allow-check-signing", "B3-06: whether an open document can be signed, by document id"],
  [
    "allow-choose-signing-file",
    "B3-06: a certificate file (PKCS #12) to sign with; the main process asks in its open dialog and keeps the file, the page gets a number and the file's name",
  ],
  [
    "allow-list-signing-certificates",
    "B3-06, ADR 0020: the certificates of the user's own store that have a private key (in Windows or on a smart card), read only; the page gets numbers, names and dates, and no key is opened",
  ],
  [
    "allow-sign-document",
    "B3-06, ADR 0020: a signed copy of an open document's file, with a certificate file (the password goes to Windows once, the private key stays in the main process and in memory) or a certificate of the user's store (its key stays in Windows or on the card, which asks for its PIN); the main process asks where, never the document's own file, and a new worker verifies the copy",
  ],
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

/** Extracts the block inside tauri::generate_handler![ ... ] from lib.rs source. */
function extractHandlerBlock(libSource) {
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

  const cleanedManifest = manifestBlock
    .replace(/\/\/.*$/gm, "")
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const buildCmds = [
    ...cleanedManifest.matchAll(/(?:"([^"]+)"|r#"([^"#]+)"#)/g),
  ].map((m) => m[1] ?? m[2]);

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

  const libSource = readFileSync(join(root, "src-tauri/src/lib.rs"), "utf8");
  const buildSource = readFileSync(join(root, "src-tauri/build.rs"), "utf8");
  problems.push(...checkCommandManifest(libSource, buildSource));

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
    "OK: CSP, dev CSP, WebView2 arguments, capabilities, and command manifest are within policy.",
  );
}
