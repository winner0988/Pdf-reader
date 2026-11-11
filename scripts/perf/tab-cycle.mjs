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
const outFile = path.resolve(option("--out-file", path.join(outDir, "tab-cycle.json")));
const heapSnapshot = process.argv.includes("--heap-snapshot");
const heapSnapshotPrefix = option("--heap-snapshot-prefix", path.join(outDir, "heap"));

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

/** Private memory in MB: the main process, WebView2 (categorized by --type=), and each worker. */
const memoryScript = `
  $appProc = $tree | Where-Object { $_.Name -eq 'pdf-reader.exe' }
  $appMem = ($appProc | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  if (-not $appMem) { $appMem = 0 }
  $wvProcs = @($tree | Where-Object { $_.Name -like 'msedgewebview2*' })
  $wvMem = ($wvProcs | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  if (-not $wvMem) { $wvMem = 0 }
  $wvBrowserMem = 0
  $wvRendererMem = 0
  $wvGpuMem = 0
  $wvOtherMem = 0
  foreach ($p in $wvProcs) {
    $proc = Get-Process -Id $p.ProcessId -ErrorAction SilentlyContinue
    if (-not $proc) { continue }
    $cmd = $p.CommandLine
    if (-not $cmd -or $cmd -notmatch '--type=') {
      $wvBrowserMem += $proc.PrivateMemorySize64
    } elseif ($cmd -match '--type=renderer') {
      $wvRendererMem += $proc.PrivateMemorySize64
    } elseif ($cmd -match '--type=gpu-process') {
      $wvGpuMem += $proc.PrivateMemorySize64
    } else {
      $wvOtherMem += $proc.PrivateMemorySize64
    }
  }
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
    mainBytes = $appMem
    webView2 = [math]::Round($wvMem / 1MB)
    webView2Details = [PSCustomObject]@{
      total = [math]::Round($wvMem / 1MB)
      browser = [math]::Round($wvBrowserMem / 1MB)
      renderer = [math]::Round($wvRendererMem / 1MB)
      gpu = [math]::Round($wvGpuMem / 1MB)
      other = [math]::Round($wvOtherMem / 1MB)
    }
    workers = $workerMemList
    workersTotal = [math]::Round($workerTotal / 1MB)
    mainAndWorkers = [math]::Round(($appMem + $workerTotal) / 1MB)
    all = [math]::Round($every / 1MB)
  } | ConvertTo-Json -Compress
`;

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
let heapSnapshotStream = null;

ws.onmessage = (message) => {
  const reply = JSON.parse(message.data);
  if (reply.method === "HeapProfiler.addHeapSnapshotChunk" && heapSnapshotStream) {
    heapSnapshotStream.write(reply.params.chunk);
    return;
  }
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

async function collectGarbage() {
  try {
    await send("HeapProfiler.collectGarbage");
    await sleep(100);
    await send("HeapProfiler.collectGarbage");
    await sleep(200);
    return "ok";
  } catch (error) {
    console.warn("HeapProfiler.collectGarbage failed:", error);
    return "failed";
  }
}

async function snapshot() {
  const json = powershell(treeScript + memoryScript);
  const snap = JSON.parse(json);

  let domCounters = null;
  try {
    const reply = await send("Memory.getDOMCounters");
    const res = reply.result ?? reply;
    domCounters = {
      documents: res.documents ?? 0,
      nodes: res.nodes ?? 0,
      jsEventListeners: res.jsEventListeners ?? 0,
    };
  } catch (err) {
    console.warn("Memory.getDOMCounters failed:", err);
  }

  let jsHeap = null;
  try {
    const reply = await send("Runtime.getHeapUsage");
    const res = reply.result ?? reply;
    jsHeap = {
      usedMB: Math.round((res.usedSize ?? 0) / 1024 / 1024),
      totalMB: Math.round((res.totalSize ?? 0) / 1024 / 1024),
      usedBytes: res.usedSize ?? 0,
      totalBytes: res.totalSize ?? 0,
    };
  } catch (err) {
    console.warn("Runtime.getHeapUsage failed:", err);
  }

  return {
    ...snap,
    domCounters,
    jsHeap,
  };
}

async function takeHeapSnapshot(filePath) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  const stream = fs.createWriteStream(filePath, "utf8");
  heapSnapshotStream = stream;
  try {
    await send("HeapProfiler.enable");
    await send("HeapProfiler.takeHeapSnapshot", { reportProgress: false });
  } finally {
    heapSnapshotStream = null;
    stream.end();
    await new Promise((resolve) => stream.on("finish", resolve));
  }
}

const helpers = `(() => {
  if (window.__perf) return true;
  const getBaseCanvas = () => {
    const all = [...document.querySelectorAll('main[data-region="canvas"]')];
    if (window.__baseDocId) {
      const match = all.find((m) => m.querySelector('[data-doc="' + window.__baseDocId + '"]'));
      if (match) return match;
    }
    return all[0] ?? null;
  };
  const getCycleCanvas = () => {
    const base = getBaseCanvas();
    return [...document.querySelectorAll('main[data-region="canvas"]')].find((m) => m !== base) ?? null;
  };
  const isBaseHidden = () => {
    const c = getBaseCanvas();
    if (!c) return true;
    const p = c.closest('[role="tabpanel"]');
    return (p && p.hidden) || c.getClientRects().length === 0;
  };
  const isBaseVisible = () => {
    const c = getBaseCanvas();
    if (!c) return false;
    const p = c.closest('[role="tabpanel"]');
    return !(p && p.hidden) && c.getClientRects().length > 0;
  };
  const isCycleVisibleAndReady = () => {
    const c = getCycleCanvas();
    if (!c) return false;
    const p = c.closest('[role="tabpanel"]');
    if (!p || p.hidden || c.getClientRects().length === 0) return false;
    const slot = c.querySelector('[role="img"][aria-label="第 1 頁"]');
    if (!slot || slot.dataset.state !== 'ready') return false;
    const canvas = slot.querySelector('canvas');
    return (canvas?.width ?? 0) > 1;
  };
  const drawn = (el) => el?.dataset.state === 'ready' && (el.querySelector('canvas')?.width ?? 0) > 1;
  const slots = () => [...(getBaseCanvas()?.querySelectorAll('[role="img"][data-doc]') ?? [])].filter((el) => el.getClientRects().length > 0);
  Object.assign(window, {
    __getBaseCanvas: getBaseCanvas,
    __getCycleCanvas: getCycleCanvas,
    __isBaseHidden: isBaseHidden,
    __isBaseVisible: isBaseVisible,
    __isCycleVisibleAndReady: isCycleVisibleAndReady,
    __getBaseScrollTop: () => getBaseCanvas()?.scrollTop ?? 0,
    __scrollCycleCanvas: (delta = 1200) => {
      const c = getCycleCanvas();
      if (c) {
        c.scrollTop += delta;
        return true;
      }
      return false;
    },
    __slot: (n) => slots().find((el) => el.getAttribute('aria-label') === '第 ' + n + ' 頁'),
    __ready: (n) => drawn(window.__slot(n)),
    __doc: () => slots()[0]?.dataset.doc ?? '',
  });
  window.__perf = true;
  return true;
})()`;

// Stalled safety timeout
setTimeout(() => fail("Stopped after timeout."), Math.max(900_000, totalCycles * 10_000)).unref();

// Wait for base document first page
while (performance.now() - launched < 60_000) {
  try {
    if (await evaluate(`document.readyState === 'complete' && ${helpers} && window.__ready(1)`)) break;
  } catch {}
  await sleep(20);
}
const baseDocId = await evaluate(`window.__doc()`);
if (!baseDocId) fail("Failed to open base document.");
await evaluate(`window.__baseDocId = ${JSON.stringify(baseDocId)}`);

console.log(`Base document ready (docId: ${baseDocId}). Running ${totalCycles} tab cycles...`);

const checkpoints = [];
const t0 = performance.now();

// Initial checkpoint
const initialGc = await collectGarbage();
if (heapSnapshot) {
  console.log("Taking initial heap snapshot...");
  await takeHeapSnapshot(`${heapSnapshotPrefix}-cycle-0.heapsnapshot`);
  console.log(`Saved initial heap snapshot to ${heapSnapshotPrefix}-cycle-0.heapsnapshot`);
}
const initial = await snapshot();
checkpoints.push({ cycle: 0, elapsedSec: 0, gc: initialGc, ...initial });
console.log(
  `[Cycle 0] Workers: ${initial.workerCount}, Main: ${initial.main} MB, WebView2: ${initial.webView2} MB ` +
  `(r:${initial.webView2Details.renderer}/b:${initial.webView2Details.browser}/g:${initial.webView2Details.gpu}/o:${initial.webView2Details.other}), ` +
  `WorkersTotal: ${initial.workersTotal} MB, Total: ${initial.all} MB, DOM: ${initial.domCounters?.nodes} nodes, JS Heap: ${initial.jsHeap?.usedMB} MB`
);

for (let i = 1; i <= totalCycles; i++) {
  // 每一輪記下基準文件的捲動位置
  const baseScrollBefore = await evaluate("window.__getBaseScrollTop()");

  // Open file in running app (new tab)
  openInRunningApp(cycleFile);

  // 開檔之後，先等新分頁的畫布成為看得見的那一個（基準文件的那一個看不見了），再捲動新分頁
  const openedMs = await until(
    "window.__isBaseHidden() && window.__isCycleVisibleAndReady()",
    10_000,
  );
  if (openedMs < 0) {
    const diag = await evaluate(`({
      baseHidden: window.__isBaseHidden(),
      cycleReady: window.__isCycleVisibleAndReady(),
      baseVisible: window.__isBaseVisible(),
      canvasCount: document.querySelectorAll('main[data-region="canvas"]').length,
      tabCount: document.querySelectorAll('div[role="tablist"][aria-label="已開啟的文件"] [role="tab"]').length
    })`).catch((e) => String(e));
    fail(`Cycle ${i}: new tab did not open, become visible, or render page 1. Diag: ${JSON.stringify(diag)}`);
  }

  // 捲動新分頁
  await evaluate("window.__scrollCycleCanvas(1200)");
  await sleep(100);

  // Close the new tab via its exact close button
  const cycleBaseName = path.basename(cycleFile);
  await evaluate(`(() => {
    const btn = document.querySelector('button[aria-label="關閉「${cycleBaseName}」"]');
    if (btn) btn.click();
  })()`);

  const closedMs = await until(
    `document.querySelectorAll('div[role="tablist"][aria-label="已開啟的文件"] [role="tab"]').length === 1 && window.__isBaseVisible()`,
    10_000,
  );
  if (closedMs < 0) {
    const diag = await evaluate(`({
      doc: window.__doc(),
      baseDocId: ${JSON.stringify(baseDocId)},
      baseVisible: window.__isBaseVisible(),
      docTabCount: document.querySelectorAll('div[role="tablist"][aria-label="已開啟的文件"] [role="tab"]').length
    })`).catch((e) => String(e));
    console.error("DIAGNOSTIC ON FAILURE:", JSON.stringify(diag));
    fail(`Cycle ${i}: failed to close tab back to base document.`);
  }

  // 每一輪記下基準文件的捲動位置。開關前後要一樣，不一樣就讓腳本失敗，並印出是第幾輪
  const baseScrollAfter = await evaluate("window.__getBaseScrollTop()");
  if (baseScrollAfter !== baseScrollBefore) {
    fail(`Cycle ${i}: 基準文件捲動位置在開關前後不一致 (第 ${i} 輪：開前 ${baseScrollBefore} px，關後 ${baseScrollAfter} px)`);
  }

  // Short pause to allow Windows job/process cleanup
  await sleep(100);

  if (i % 10 === 0) {
    const gc = await collectGarbage();
    const snap = await snapshot();
    const elapsedSec = Math.round((performance.now() - t0) / 1000);
    checkpoints.push({ cycle: i, elapsedSec, gc, ...snap });
    console.log(
      `[Cycle ${i}/${totalCycles}] (${elapsedSec}s) Workers: ${snap.workerCount}, Main: ${snap.main} MB, WebView2: ${snap.webView2} MB ` +
      `(r:${snap.webView2Details.renderer}/b:${snap.webView2Details.browser}/g:${snap.webView2Details.gpu}/o:${snap.webView2Details.other}), ` +
      `WorkersTotal: ${snap.workersTotal} MB, Total: ${snap.all} MB, DOM: ${snap.domCounters?.nodes} nodes, JS Heap: ${snap.jsHeap?.usedMB} MB`
    );
  }
}

console.log(`Completed ${totalCycles} cycles. Waiting 30 seconds for memory cleanup / garbage collection...`);
await sleep(30_000);
const finalGc = await collectGarbage();

if (heapSnapshot) {
  console.log("Taking final heap snapshot...");
  await takeHeapSnapshot(`${heapSnapshotPrefix}-final.heapsnapshot`);
  console.log(`Saved final heap snapshot to ${heapSnapshotPrefix}-final.heapsnapshot`);
}

const final = await snapshot();
const totalElapsedSec = Math.round((performance.now() - t0) / 1000);
checkpoints.push({ cycle: "final (after 30s)", elapsedSec: totalElapsedSec, gc: finalGc, ...final });
console.log(
  `[Final] Workers: ${final.workerCount}, Main: ${final.main} MB, WebView2: ${final.webView2} MB ` +
  `(r:${final.webView2Details.renderer}/b:${final.webView2Details.browser}/g:${final.webView2Details.gpu}/o:${final.webView2Details.other}), ` +
  `WorkersTotal: ${final.workersTotal} MB, Total: ${final.all} MB, DOM: ${final.domCounters?.nodes} nodes, JS Heap: ${final.jsHeap?.usedMB} MB`
);

ws.close();
cleanUp();

// Judgement criteria:
// 1. Worker count returns to 1 (only the base document worker remains).
// 2. Main process memory returns close to initial without dimension patching (#510).
//    (Based on 3 runs of 100 cycles where max delta was +2 MB, threshold set to initial + 15 MB).
// 3. WebView2 memory returns close to initial (accounting for GPU process cache variance up to 90 MB
//    and renderer/browser process memory across cycles, threshold set to initial + 100 MB).
const mainThresholdMB = initial.main + 15;
const webviewThresholdMB = initial.webView2 + 100;

const workerCountOk = final.workerCount === 1;
const mainMemoryOk = final.main <= mainThresholdMB;
const webView2MemoryOk = final.webView2 <= webviewThresholdMB;

const result = {
  totalCycles,
  totalElapsedSec,
  thresholds: {
    mainMaxMB: Math.round(mainThresholdMB),
    webView2MaxMB: Math.round(webviewThresholdMB),
    reason: "#491 的量測（2026-11-08）確認開關分頁本身不殘留點陣快取；#510 修正隱藏分頁量版面問題後，腳本不改寫尺寸，基準文件捲動位置在開關前後保持一致且主行程回到接近初始值（3 次 100 輪最大增加 2 MB，門檻定為開始 + 15 MB）；WebView2 門檻定為開始 + 100 MB，容納 GPU 行程自然快取達 90 MB 之浮動與行程增長",
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

fs.writeFileSync(outFile, JSON.stringify(result, null, 2));

console.log(`\n--- Tab Cycle ${totalCycles} Summary ---`);
console.log(`${workerCountOk ? "ok  " : "MISS"} Worker count: ${final.workerCount} (target 1)`);
console.log(`${mainMemoryOk ? "ok  " : "MISS"} Main memory: ${final.main} MB (initial ${initial.main} MB, target <= ${Math.round(mainThresholdMB)} MB)`);
console.log(`${webView2MemoryOk ? "ok  " : "MISS"} WebView2 memory: ${final.webView2} MB (initial ${initial.webView2} MB, target <= ${Math.round(webviewThresholdMB)} MB)`);
console.log(`Overall: ${result.judgement.allPassed ? "PASSED" : "FAILED"}`);
