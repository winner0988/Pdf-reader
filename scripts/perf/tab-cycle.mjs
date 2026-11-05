// Development-only performance measurement: opening and closing tabs 100 times (B4-10, Issue #418).
// Never shipped, never run on CI.
//
// It opens a base document, then repeatedly opens another document in a new tab,
// waits for page 1 to be ready, scrolls a few pages, closes the tab, and repeats 100 times.
// Memory and worker counts are sampled before start, every 10 cycles, and 30 seconds after completion.
//
//   node scripts/perf/tab-cycle.mjs [--cycles 100] [--out <folder>]

import { execFileSync, spawn } from "node:child_process";
import fs from "node:fs";
import { createServer } from "node:net";
import os from "node:os";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "../..");
const option = (name, fallback) => {
  const at = process.argv.indexOf(name);
  return at >= 0 && process.argv[at + 1] ? process.argv[at + 1] : fallback;
};

const targetDir = process.env.CARGO_TARGET_DIR ?? path.join(ROOT, "target");
const app = path.resolve(option("--app", path.join(targetDir, "release", "pdf-reader.exe")));
const outDir = path.resolve(option("--out", path.join(ROOT, "test-results", "perf")));
const baseFile = path.resolve(option("--base", path.join(ROOT, "tests", "corpus", "large", "output", "large-1000-pages.pdf")));
const cycleFile = path.resolve(option("--cycle-file", path.join(ROOT, "tests", "corpus", "large", "output", "large-text-1000-pages.pdf")));
const totalCycles = Number(option("--cycles", 100));

for (const file of [app, baseFile, cycleFile]) {
  if (!fs.existsSync(file)) {
    console.error(`Missing ${file}: see the commands at the top of this script.`);
    process.exit(1);
  }
}
fs.mkdirSync(outDir, { recursive: true });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function freePort() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

// ---------------------------------------------------------------- the app and its processes

const port = await freePort();
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "pdf-reader-perf-tab-"));
const data = fs.mkdtempSync(path.join(os.tmpdir(), "pdf-reader-perf-tab-data-"));
const env = {
  ...process.env,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  WEBVIEW2_USER_DATA_FOLDER: profile,
  PDF_READER_DATA_DIR: data,
};
const launched = performance.now();
const child = spawn(app, [baseFile], { env, stdio: "ignore" });
let exited = false;
child.on("exit", () => (exited = true));

function cleanUp() {
  try {
    execFileSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  } catch {}
  for (const folder of [profile, data]) {
    try {
      fs.rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    } catch {}
  }
}

function fail(message) {
  console.error(message);
  cleanUp();
  process.exit(1);
}

/** PowerShell that collects the app and every process under it in `$tree`. */
const treeScript = `
  $all = Get-CimInstance Win32_Process
  $ids = @(${child.pid})
  do {
    $before = $ids.Count
    $ids = @($ids + ($all | Where-Object { $ids -contains $_.ParentProcessId } | ForEach-Object { $_.ProcessId }) | Select-Object -Unique)
  } while ($ids.Count -gt $before)
  $tree = $all | Where-Object { $ids -contains $_.ProcessId }
`;
const powershell = (script) =>
  execFileSync("powershell.exe", ["-NoProfile", "-Command", script], { encoding: "utf8" }).trim();

/** Private memory in MB: the main process, WebView2, and each worker. */
const memoryScript = `
  $appProc = $tree | Where-Object { $_.Name -eq 'pdf-reader.exe' }
  $appMem = ($appProc | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  if (-not $appMem) { $appMem = 0 }
  $wvProcs = $tree | Where-Object { $_.Name -like 'msedgewebview2*' }
  $wvMem = ($wvProcs | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  if (-not $wvMem) { $wvMem = 0 }
  $workerProcs = @($tree | Where-Object { $_.Name -eq 'pdf_worker.exe' })
  $workerMemList = @($workerProcs | ForEach-Object {
    $p = Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue
    if ($p) { [PSCustomObject]@{ pid = $_.ProcessId; memoryMB = [math]::Round($p.PrivateMemorySize64 / 1MB) } }
  })
  $workerTotal = ($workerProcs | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  if (-not $workerTotal) { $workerTotal = 0 }
  $every = ($tree | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  if (-not $every) { $every = 0 }
  [PSCustomObject]@{
    workerCount = $workerProcs.Count
    main = [math]::Round($appMem / 1MB)
    webView2 = [math]::Round($wvMem / 1MB)
    workers = $workerMemList
    workersTotal = [math]::Round($workerTotal / 1MB)
    mainAndWorkers = [math]::Round(($appMem + $workerTotal) / 1MB)
    all = [math]::Round($every / 1MB)
  } | ConvertTo-Json -Compress
`;

function snapshot() {
  const json = powershell(treeScript + memoryScript);
  return JSON.parse(json);
}

function openInRunningApp(file) {
  spawn(app, [file], { env, stdio: "ignore" });
}

// ---------------------------------------------------------------- DevTools

let target;
for (let i = 0; i < 240 && !target; i++) {
  if (exited) fail("The app quit at once: a build with the same identifier is probably running already.");
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    target = list.find((t) => t.type === "page");
  } catch {}
  if (!target) await sleep(100);
}
if (!target) fail(`No WebView2 page on port ${port}.`);

const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolve) => (ws.onopen = resolve));
let lastId = 0;
const pending = new Map();

ws.onmessage = (message) => {
  const reply = JSON.parse(message.data);
  if (reply.id && pending.has(reply.id)) pending.get(reply.id)(reply);
};

const send = (method, params = {}) =>
  new Promise((resolve) => {
    const id = ++lastId;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });

const evaluate = async (expression) => {
  const reply = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (reply.result?.exceptionDetails) throw new Error(JSON.stringify(reply.result.exceptionDetails).slice(0, 400));
  return reply.result?.result?.value;
};

async function until(expression, timeoutMs = 30_000) {
  const t0 = performance.now();
  while (performance.now() - t0 < timeoutMs) {
    try {
      if (await evaluate(expression)) return Math.round(performance.now() - t0);
    } catch {}
    await sleep(20);
  }
  return -1;
}

async function pressCtrl(key, keyCode) {
  const event = { key, code: `Key${key.toUpperCase()}`, windowsVirtualKeyCode: keyCode, modifiers: 2 };
  await send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...event });
  await send("Input.dispatchKeyEvent", { type: "keyUp", ...event });
}

const helpers = `(() => {
  if (window.__perf) return true;
  const scroller = () => [...document.querySelectorAll('main[data-region="canvas"]')].find((m) => m.getClientRects().length > 0);
  const slots = () => [...(scroller()?.querySelectorAll('[role="img"][data-doc]') ?? [])].filter((el) => el.getClientRects().length > 0);
  const drawn = (el) => el?.dataset.state === 'ready' && (el.querySelector('canvas')?.width ?? 0) > 1;
  Object.assign(window, {
    __scroller: scroller,
    __slot: (n) => slots().find((el) => el.getAttribute('aria-label') === '第 ' + n + ' 頁'),
    __ready: (n) => drawn(window.__slot(n)),
    __doc: () => slots()[0]?.dataset.doc ?? '',
  });
  window.__perf = true;
  return true;
})()`;

// Stalled safety timeout
setTimeout(() => fail("Stopped after 15 minutes."), 900_000).unref();

// Wait for base document first page
while (performance.now() - launched < 60_000) {
  try {
    if (await evaluate(`document.readyState === 'complete' && ${helpers} && window.__ready(1)`)) break;
  } catch {}
  await sleep(20);
}
const baseDocId = await evaluate(`window.__doc()`);
if (!baseDocId) fail("Failed to open base document.");

console.log(`Base document ready (docId: ${baseDocId}). Running ${totalCycles} tab cycles...`);

const checkpoints = [];
const t0 = performance.now();

// Initial checkpoint
const initial = snapshot();
checkpoints.push({ cycle: 0, elapsedSec: 0, ...initial });
console.log(`[Cycle 0] Workers: ${initial.workerCount}, Main: ${initial.main} MB, WebView2: ${initial.webView2} MB, WorkersTotal: ${initial.workersTotal} MB, Total: ${initial.all} MB`);

for (let i = 1; i <= totalCycles; i++) {
  // Open file in running app (new tab)
  openInRunningApp(cycleFile);
  const openedMs = await until(`window.__doc() !== ${JSON.stringify(baseDocId)} && window.__ready(1)`, 10_000);
  if (openedMs < 0) fail(`Cycle ${i}: new tab did not open or render page 1.`);

  // Scroll a few pages
  await evaluate(`(() => { const s = window.__scroller(); if (s) s.scrollTop += 1200; })()`);
  await sleep(100);

  // Close the new tab via its exact close button
  const cycleBaseName = path.basename(cycleFile);
  await evaluate(`(() => {
    const btn = document.querySelector('button[aria-label="關閉「${cycleBaseName}」"]');
    if (btn) btn.click();
  })()`);

  const closedMs = await until(
    `document.querySelectorAll('div[role="tablist"][aria-label="已開啟的文件"] [role="tab"]').length === 1 && window.__doc() === ${JSON.stringify(baseDocId)}`,
    10_000,
  );
  if (closedMs < 0) {
    const diag = await evaluate(`({
      doc: window.__doc(),
      baseDocId: ${JSON.stringify(baseDocId)},
      docTabCount: document.querySelectorAll('div[role="tablist"][aria-label="已開啟的文件"] [role="tab"]').length
    })`).catch((e) => String(e));
    console.error("DIAGNOSTIC ON FAILURE:", JSON.stringify(diag));
    fail(`Cycle ${i}: failed to close tab back to base document.`);
  }

  // Short pause to allow Windows job/process cleanup
  await sleep(100);

  if (i % 10 === 0) {
    const snap = snapshot();
    const elapsedSec = Math.round((performance.now() - t0) / 1000);
    checkpoints.push({ cycle: i, elapsedSec, ...snap });
    console.log(`[Cycle ${i}/${totalCycles}] (${elapsedSec}s) Workers: ${snap.workerCount}, Main: ${snap.main} MB, WebView2: ${snap.webView2} MB, WorkersTotal: ${snap.workersTotal} MB, Total: ${snap.all} MB`);
  }
}

console.log("Completed 100 cycles. Waiting 30 seconds for memory cleanup / garbage collection...");
await sleep(30_000);

const final = snapshot();
const totalElapsedSec = Math.round((performance.now() - t0) / 1000);
checkpoints.push({ cycle: "final (after 30s)", elapsedSec: totalElapsedSec, ...final });
console.log(`[Final] Workers: ${final.workerCount}, Main: ${final.main} MB, WebView2: ${final.webView2} MB, WorkersTotal: ${final.workersTotal} MB, Total: ${final.all} MB`);

ws.close();
cleanUp();

// Judgement criteria:
// 1. Worker count returns to 1 (only the base document worker remains).
// 2. Main process memory: cycle 0 is a cold start with an empty raster cache (~55 MB).
//    During tab cycles, the raster cache (DEFAULT_CACHE_BYTES = 256 MB in render.rs) populates,
//    and the MSVC CRT heap retains committed virtual memory pages for subsequent allocations.
//    The memory must stabilize within the cache budget and not grow unboundedly (initial + 256 MB cache + margin <= 350 MB).
// 3. WebView2 memory returns close to initial: within initial + 100 MB (allowing for Chromium V8 DOM/JIT pool).
const mainThresholdMB = Math.max(initial.main + 276, 350);
const webviewThresholdMB = Math.max(initial.webView2 * 1.25, initial.webView2 + 100);

const workerCountOk = final.workerCount === 1;
const mainMemoryOk = final.main <= mainThresholdMB;
const webView2MemoryOk = final.webView2 <= webviewThresholdMB;

const result = {
  totalCycles,
  totalElapsedSec,
  thresholds: {
    mainMaxMB: Math.round(mainThresholdMB),
    webView2MaxMB: Math.round(webviewThresholdMB),
    reason: "Main process includes DEFAULT_CACHE_BYTES = 256 MB raster cache plus retained heap; WebView2 retains V8 JIT pool within 100 MB after GC.",
  },
  initial,
  final,
  judgement: {
    workerCountReturnedToOne: { met: workerCountOk, initial: initial.workerCount, final: final.workerCount, expected: 1 },
    mainMemoryNearInitial: { met: mainMemoryOk, initialMB: initial.main, finalMB: final.main, thresholdMB: Math.round(mainThresholdMB) },
    webView2MemoryNearInitial: { met: webView2MemoryOk, initialMB: initial.webView2, finalMB: final.webView2, thresholdMB: Math.round(webviewThresholdMB) },
    allPassed: workerCountOk && mainMemoryOk && webView2MemoryOk,
  },
  checkpoints,
};

fs.writeFileSync(path.join(outDir, "tab-cycle.json"), JSON.stringify(result, null, 2));

console.log("\n--- Tab Cycle 100 Summary ---");
console.log(`${workerCountOk ? "ok  " : "MISS"} Worker count: ${final.workerCount} (target 1)`);
console.log(`${mainMemoryOk ? "ok  " : "MISS"} Main memory: ${final.main} MB (initial ${initial.main} MB, target <= ${Math.round(mainThresholdMB)} MB)`);
console.log(`${webView2MemoryOk ? "ok  " : "MISS"} WebView2 memory: ${final.webView2} MB (initial ${initial.webView2} MB, target <= ${Math.round(webviewThresholdMB)} MB)`);
console.log(`Overall: ${result.judgement.allPassed ? "PASSED" : "FAILED"}`);
