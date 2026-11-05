// Development-only performance measurement: soak test for prolonged scrolling and zooming (B4-10, Issue #418).
// Never shipped, never run on CI.
//
// It opens a large document (default 2,000 pages, ~300 MB), continually performs realistic
// scrolling and zooming operations, and records process memory every minute.
// At the end, it writes soak.json and soak.csv, and checks whether memory continues to grow indefinitely.
//
//   node scripts/perf/soak.mjs [--minutes 120] [--file <path>] [--out <folder>]

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
const defaultFile = fs.existsSync(path.join(ROOT, "tests", "corpus", "large", "output", "large-2000-pages.pdf"))
  ? path.join(ROOT, "tests", "corpus", "large", "output", "large-2000-pages.pdf")
  : path.join(ROOT, "tests", "corpus", "large", "output", "large-1000-pages.pdf");
const file = path.resolve(option("--file", defaultFile));
const totalMinutes = Number(option("--minutes", 120));

for (const f of [app, file]) {
  if (!fs.existsSync(f)) {
    console.error(`Missing ${f}: see the commands at the top of this script.`);
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
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "pdf-reader-perf-soak-"));
const data = fs.mkdtempSync(path.join(os.tmpdir(), "pdf-reader-perf-soak-data-"));
const env = {
  ...process.env,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  WEBVIEW2_USER_DATA_FOLDER: profile,
  PDF_READER_DATA_DIR: data,
};
const launched = performance.now();
const child = spawn(app, [file], { env, stdio: "ignore" });
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
    __zoom: (value) => {
      const select = [...document.querySelectorAll('select[aria-label="縮放比例"]')].find((el) => el.getClientRects().length > 0);
      if (select) {
        Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(select, String(value));
        select.dispatchEvent(new Event('change', { bubbles: true }));
      }
    },
  });
  window.__perf = true;
  return true;
})()`;

// Safety timeout: total minutes + 5 minutes margin
setTimeout(() => fail("Stopped after timeout."), (totalMinutes + 5) * 60_000).unref();

// Wait for document ready
while (performance.now() - launched < 60_000) {
  try {
    if (await evaluate(`document.readyState === 'complete' && ${helpers} && window.__ready(1)`)) break;
  } catch {}
  await sleep(20);
}

console.log(`Document loaded. Starting ${totalMinutes}-minute soak test on ${path.basename(file)}...`);

const samples = [];
const csvPath = path.join(outDir, "soak.csv");
fs.writeFileSync(csvPath, "minute,timestamp,mainMB,webView2MB,workersTotalMB,mainAndWorkersMB,allMB\n");

const startTime = Date.now();
const endTime = startTime + totalMinutes * 60_000;
let nextSampleTime = startTime + 60_000;
let minuteCount = 0;

// Initial sample at minute 0
const initial = snapshot();
samples.push({ minute: 0, timestamp: new Date().toISOString(), ...initial });
fs.appendFileSync(csvPath, `0,${new Date().toISOString()},${initial.main},${initial.webView2},${initial.workersTotal},${initial.mainAndWorkers},${initial.all}\n`);
console.log(`[Minute 0/${totalMinutes}] Main: ${initial.main} MB, WebView2: ${initial.webView2} MB, Workers: ${initial.workersTotal} MB, Total: ${initial.all} MB`);

// User interaction patterns
const zoomLevels = ["fitWidth", "100", "150", "200", "fitWidth", "125"];
let zoomIdx = 0;
let stepCounter = 0;

while (Date.now() < endTime) {
  stepCounter++;
  const action = stepCounter % 6;

  try {
    if (action === 0) {
      // Zoom
      const zoom = zoomLevels[zoomIdx % zoomLevels.length];
      zoomIdx++;
      await evaluate(`window.__zoom(${JSON.stringify(zoom)})`);
      await sleep(1000);
    } else if (action === 1 || action === 2 || action === 4) {
      // Scroll down
      await evaluate(`(() => { const s = window.__scroller(); if (s) s.scrollTop += 1200; })()`);
      await sleep(800);
    } else if (action === 3) {
      // Scroll up
      await evaluate(`(() => { const s = window.__scroller(); if (s) s.scrollTop = Math.max(0, s.scrollTop - 800); })()`);
      await sleep(800);
    } else {
      // Periodic jump to middle or top if scrolled far
      await evaluate(`(() => {
        const s = window.__scroller();
        if (s && s.scrollTop > s.scrollHeight * 0.8) s.scrollTop = 0;
      })()`);
      await sleep(600);
    }
  } catch (err) {
    // Ignore transient eval during animation
  }

  // Check if 1 minute elapsed for periodic sample
  if (Date.now() >= nextSampleTime) {
    minuteCount++;
    nextSampleTime += 60_000;
    const snap = snapshot();
    const timestamp = new Date().toISOString();
    samples.push({ minute: minuteCount, timestamp, ...snap });
    fs.appendFileSync(csvPath, `${minuteCount},${timestamp},${snap.main},${snap.webView2},${snap.workersTotal},${snap.mainAndWorkers},${snap.all}\n`);
    console.log(`[Minute ${minuteCount}/${totalMinutes}] Main: ${snap.main} MB, WebView2: ${snap.webView2} MB, Workers: ${snap.workersTotal} MB, Total: ${snap.all} MB`);
  }
}

ws.close();
cleanUp();

// Criterion: memory must not keep growing unbounded.
// For full soak (>= 60 min): average of last 30 minutes must not exceed 110% of min 30-60 average.
// For short test runs (< 60 min, e.g. 5 min test): final minute memory must not exceed minute 2 (warm baseline) * 1.15.
let ruleDescription = "";
let passed = false;
let comparison = {};

if (totalMinutes >= 60) {
  ruleDescription = "Average main+workers of the last 30 minutes must not exceed 110% of the average between minutes 30 and 60.";
  const midSlice = samples.filter((s) => s.minute >= 30 && s.minute <= 60);
  const endSlice = samples.filter((s) => s.minute >= totalMinutes - 30);
  const avgMid = midSlice.reduce((sum, s) => sum + s.mainAndWorkers, 0) / (midSlice.length || 1);
  const avgEnd = endSlice.reduce((sum, s) => sum + s.mainAndWorkers, 0) / (endSlice.length || 1);
  const threshold = avgMid * 1.10;
  passed = avgEnd <= threshold;
  comparison = { avgMid: Math.round(avgMid), avgEnd: Math.round(avgEnd), threshold: Math.round(threshold), passed };
} else {
  ruleDescription = "For test run (<60 min): final minute main+workers must not exceed minute 2 warm baseline * 1.15.";
  const warmSample = samples.find((s) => s.minute === 2) || samples[1] || samples[0];
  const finalSample = samples[samples.length - 1];
  const threshold = warmSample.mainAndWorkers * 1.15;
  passed = finalSample.mainAndWorkers <= threshold;
  comparison = { warmBaselineMB: warmSample.mainAndWorkers, finalMB: finalSample.mainAndWorkers, threshold: Math.round(threshold), passed };
}

const report = {
  totalMinutes,
  file: path.basename(file),
  rule: ruleDescription,
  judgement: { met: passed, ...comparison },
  samples,
};

fs.writeFileSync(path.join(outDir, "soak.json"), JSON.stringify(report, null, 2));

console.log("\n--- Soak Test Summary ---");
console.log(`Duration: ${totalMinutes} minutes`);
console.log(`Rule: ${ruleDescription}`);
console.log(`Verdict: ${passed ? "PASSED" : "FAILED"}`);
console.log(`Results saved to ${path.join(outDir, "soak.json")} and ${csvPath}`);
