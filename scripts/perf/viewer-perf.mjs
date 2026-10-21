// Development-only measurement for MVP-07 and issue #240 (docs/architecture/rendering.md#端對端量測).
// Never shipped. It drives a release build that the developer started by hand with WebView2 remote
// debugging on a local port, and talks only to that port on 127.0.0.1 (Chrome DevTools Protocol).
// It opens nothing on the network.
//
// Usage:
//   python tests/corpus/generate.py --large
//   $env:CARGO_TARGET_DIR="C:/Users/UserB/AppData/Local/Temp/target-240"
//   cargo build --release -p pdf_worker && pnpm tauri build --no-bundle
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9240"
//   $env:CARGO_TARGET_DIR/release/pdf-reader.exe tests/corpus/large/output/large-1000-pages.pdf
//   node scripts/perf/viewer-perf.mjs 9240 <folder for screenshots> <appExe> <a0Pdf>
//
// Measures targets T1 - T7:
//   T1: Open to first page (large-1000-pages.pdf) <= 1.0 s
//   T2: Normal scrolling blank time = 0 ms
//   T3: Fast scroll stop to all visible pages rendered <= 500 ms
//   T4: Zoom (100% -> 200%): old bitmap <= 1 frame; new crisp raster <= 400 ms
//   T5: Large page (A0 poster, 200% zoom): render <= 1.5 s; main + worker memory < 1 GB
//   T6: Fast scroll 10s memory < 1 GB
//   T7: Document idle 10s CPU < 1%
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const [
  port = "9240",
  outDir = ".",
  appExe = "target/release/pdf-reader.exe",
  a0Path = "tests/corpus/large/output/large-a0-poster.pdf",
] = process.argv.slice(2);

fs.mkdirSync(outDir, { recursive: true });

function killOurWorker() {
  try {
    execFileSync("powershell.exe", ["-NoProfile", "-Command", `
      $conn = Get-NetTCPConnection -LocalPort ${port} -ErrorAction SilentlyContinue | Select-Object -First 1
      if ($conn) {
        $wvPid = $conn.OwningProcess
        $wv = Get-CimInstance Win32_Process -Filter "ProcessId = $wvPid" -ErrorAction SilentlyContinue
        $readerPid = if ($wv) { $wv.ParentProcessId } else { $null }
        if ($readerPid) {
          $workers = Get-CimInstance Win32_Process -Filter "Name = 'pdf_worker.exe'" | Where-Object { $_.ParentProcessId -eq $readerPid }
          foreach ($w in $workers) {
            Stop-Process -Id $w.ProcessId -Force -ErrorAction SilentlyContinue
          }
        }
      }
    `], { stdio: "ignore" });
  } catch {}
}

function sampleOurMemoryMB() {
  try {
    const out = execFileSync("powershell.exe", ["-NoProfile", "-Command", `
      $conn = Get-NetTCPConnection -LocalPort ${port} -ErrorAction SilentlyContinue | Select-Object -First 1
      $readerPid = if ($conn) { (Get-CimInstance Win32_Process -Filter "ProcessId = $($conn.OwningProcess)").ParentProcessId } else { $null }
      $procs = @()
      if ($readerPid) {
        $procs += Get-Process -Id $readerPid -ErrorAction SilentlyContinue
        $workers = Get-CimInstance Win32_Process -Filter "Name = 'pdf_worker.exe'" | Where-Object { $_.ParentProcessId -eq $readerPid }
        foreach ($w in $workers) { $procs += Get-Process -Id $w.ProcessId -ErrorAction SilentlyContinue }
      }
      if (-not $procs) { $procs = Get-Process pdf-reader, pdf_worker -ErrorAction SilentlyContinue }
      $m = ($procs | Measure-Object PrivateMemorySize64 -Sum).Sum
      [math]::Round($m / 1MB)
    `], { encoding: "utf8" });
    return parseInt(out.trim(), 10) || 0;
  } catch {
    return 0;
  }
}

let page;
for (let i = 0; i < 120 && !page; i++) {
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    page = list.find((t) => t.type === "page");
  } catch {}
  if (!page) await new Promise((r) => setTimeout(r, 500));
}
if (!page) {
  console.error(`Could not connect to WebView2 on port ${port}`);
  process.exit(1);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) pending.get(msg.id)(msg);
};
const send = (method, params = {}) =>
  new Promise((r) => {
    const n = ++id;
    pending.set(n, r);
    ws.send(JSON.stringify({ id: n, method, params }));
  });
const evaluate = async (expression) => {
  const reply = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (reply.result?.exceptionDetails) throw new Error(JSON.stringify(reply.result.exceptionDetails).slice(0, 400));
  return reply.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const shot = async (name) => {
  const s = await send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(`${outDir}/${name}.png`, Buffer.from(s.result.data, "base64"));
};
const pct = (values, p) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted.length ? Math.round(sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * p))]) : NaN;
};

await send("Runtime.enable");
await send("Page.enable");

// Wait for the CLI-opened document.
for (let i = 0; i < 120; i++) {
  if ((await evaluate(`document.querySelector('footer')?.innerText ?? ''`)).includes("/ 1000")) break;
  await sleep(250);
}

// Instrumentation: when each page slot appears and when it becomes ready.
const initPerfScript = `(() => {
  const perf = (window.__perf = { mounted: new Map(), visibleAt: new Map(), blank: [], visibleBlank: [] });
  const getScroller = () => document.querySelector('div[role="tabpanel"]:not([hidden]) main') || document.querySelector('main');
  // Time a page is actually on screen while still blank.
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting && !perf.visibleAt.has(e.target)) perf.visibleAt.set(e.target, performance.now());
  }, { root: getScroller() });
  const note = (el) => {
    if (!(el instanceof HTMLElement) || el.getAttribute('role') !== 'img' || !el.id?.startsWith('page-')) return;
    if (el.dataset.state === 'loading' && !perf.mounted.has(el)) { perf.mounted.set(el, performance.now()); io.observe(el); }
    if (el.dataset.state === 'ready' && perf.mounted.has(el)) {
      const now = performance.now();
      perf.blank.push(now - perf.mounted.get(el));
      perf.visibleBlank.push(perf.visibleAt.has(el) ? now - perf.visibleAt.get(el) : 0);
      perf.mounted.delete(el);
      io.unobserve(el);
    }
  };
  new MutationObserver((records) => {
    for (const r of records) {
      if (r.type === 'attributes') note(r.target);
      r.addedNodes.forEach((n) => note(n));
    }
  }).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['data-state'] });
  window.__scroller = getScroller;
  window.__ready = (n) => (document.querySelector('div[role="tabpanel"]:not([hidden]) #page-' + n) || document.getElementById('page-' + n))?.dataset.state === 'ready';
  return true;
})()`;

await evaluate(initPerfScript);

const results = {};

// 1. T1: Open -> first page, warm worker and cold worker.
async function timeOpenTab() {
  const activeBefore = await evaluate(`document.querySelector('button[role="tab"][id^="tab-"][aria-selected="true"]')?.id ?? ''`);
  const docBefore = await evaluate(`(document.querySelector('div[role="tabpanel"]:not([hidden]) #page-1') || document.getElementById('page-1'))?.dataset.doc ?? ''`);
  const t0 = performance.now();
  execFileSync(appExe, [path.resolve("tests/corpus/large/output/large-1000-pages.pdf")], { stdio: "ignore" });
  while (true) {
    await sleep(20);
    const ready = await evaluate(`(() => {
      const active = document.querySelector('button[role="tab"][id^="tab-"][aria-selected="true"]')?.id;
      if (!active || active === "${activeBefore}") return false;
      const p = document.querySelector('div[role="tabpanel"]:not([hidden]) #page-1') || document.getElementById('page-1');
      if (p && p.dataset.doc && p.dataset.doc !== "${docBefore}" && p.dataset.state === 'ready') {
        const c = p.querySelector('canvas');
        return Boolean(c && c.width > 1);
      }
      return false;
    })()`);
    if (ready) return Math.round(performance.now() - t0);
    if (performance.now() - t0 > 20000) return -1;
  }
}

async function closeActiveTab() {
  await evaluate(`(() => {
    const btn = Array.from(document.querySelectorAll('button')).find((b) => b.getAttribute('aria-label')?.startsWith('關閉') && b.tabIndex === 0);
    btn?.click();
  })()`);
  for (let i = 0; i < 50; i++) {
    const active = await evaluate(`document.querySelector('button[role="tab"][id^="tab-"][aria-selected="true"]')?.id`);
    if (active === "tab-1") break;
    await sleep(100);
  }
  await sleep(300);
}

results.openWarmMs = await timeOpenTab();
await closeActiveTab();

killOurWorker();
await sleep(500);
results.openColdMs = await timeOpenTab();
await closeActiveTab();
await shot("07b-first-page");

// Re-initialize instrumentation for tab 1
await evaluate(initPerfScript);
await evaluate(`(() => { const m = document.querySelector('main'); if (m) m.scrollTop = 0; })()`);
for (let i = 0; i < 50 && !(await evaluate(`window.__ready(1)`)); i++) await sleep(100);

// 2. T2: Normal-speed scrolling: 200 px every 100 ms (about 2 pages a second) for 10 s.
await evaluate(`(() => { window.__perf.blank.length = 0; window.__perf.visibleBlank.length = 0; const m = document.querySelector('main'); if (m) m.scrollTop = 0; })()`);
await sleep(500);
await evaluate(`window.__perf.blank.length = 0; window.__perf.visibleBlank.length = 0`);
for (let i = 0; i < 100; i++) {
  await evaluate(`document.querySelector('main').scrollTop += 200`);
  await sleep(100);
}
await sleep(500);
const normal = await evaluate(`window.__perf.blank`);
const visible = await evaluate(`window.__perf.visibleBlank`);
results.normalScroll = {
  pages: normal.length,
  fromMountMsP50: pct(normal, 0.5), fromMountMsP95: pct(normal, 0.95), fromMountMsMax: pct(normal, 1),
  visibleBlankMsP50: pct(visible, 0.5), visibleBlankMsP95: pct(visible, 0.95), visibleBlankMsMax: pct(visible, 1),
};

// 3. T3: Fast scroll stop to all visible pages rendered (target <= 500 ms).
results.fastScrollStopMs = await evaluate(`(async () => {
  const s = document.querySelector('main');
  // Jump to an unrendered section in the middle of the document
  const target = Math.round(s.scrollHeight * 0.45);
  for (let step = 1; step <= 5; step++) {
    s.scrollTop = target - (5 - step) * 2000;
    await new Promise((r) => setTimeout(r, 16));
  }
  s.scrollTop = target;
  const t0 = performance.now();
  while (true) {
    await new Promise((r) => requestAnimationFrame(r));
    const scrollerRect = s.getBoundingClientRect();
    const visibleSlots = [...document.querySelectorAll('[id^=page-]')].filter((el) => {
      const r = el.getBoundingClientRect();
      return r.bottom > scrollerRect.top && r.top < scrollerRect.bottom;
    });
    if (visibleSlots.length > 0 && visibleSlots.every((el) => el.dataset.state === 'ready')) {
      return Math.round(performance.now() - t0);
    }
    if (performance.now() - t0 > 10000) return -1;
  }
})()`);

// 4. T4: Zoom 100% -> 200% (target: old bitmap <= 1 frame, new bitmap <= 400 ms).
results.zoom = await evaluate(`(async () => {
  const s = document.querySelector('main');
  s.scrollTop = 0;
  for (let i = 0; i < 50 && !window.__ready(1); i++) await new Promise((r) => setTimeout(r, 50));
  const select = document.querySelector('select[aria-label="縮放比例"]') || document.querySelector('header select');
  const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
  if (select && select.value !== "100") {
    setter.call(select, "100");
    select.dispatchEvent(new Event("change", { bubbles: true }));
  }
  // Wait for 100% render to settle
  await new Promise((r) => setTimeout(r, 1000));
  for (let i = 0; i < 50 && !window.__ready(1); i++) await new Promise((r) => setTimeout(r, 50));

  const page1 = document.querySelector('div[role="tabpanel"]:not([hidden]) #page-1') || document.getElementById('page-1');
  const canvas = page1 ? page1.querySelector('canvas') : null;
  if (!canvas) return { error: "page-1 canvas not found" };
  const initialCanvasWidth = canvas.width;
  const initialClientWidth = canvas.getBoundingClientRect().width;

  let oldBitmapFrames = -1;
  let frameCount = 0;
  let newBitmapMs = -1;

  const t0 = performance.now();
  if (select) {
    setter.call(select, "200");
    select.dispatchEvent(new Event("change", { bubbles: true }));
  } else {
    const zoomIn = document.querySelector('button[aria-label="放大"]');
    if (zoomIn) zoomIn.click();
  }

  while (performance.now() - t0 < 5000) {
    await new Promise((r) => requestAnimationFrame(r));
    frameCount++;
    const currentClientWidth = canvas.getBoundingClientRect().width;
    if (oldBitmapFrames === -1 && currentClientWidth > initialClientWidth * 1.5) {
      oldBitmapFrames = frameCount;
    }
    if (canvas.width > initialCanvasWidth * 1.5) {
      newBitmapMs = Math.round(performance.now() - t0);
      break;
    }
  }

  // Restore to fitWidth
  if (select) {
    setter.call(select, "fitWidth");
    select.dispatchEvent(new Event("change", { bubbles: true }));
  }
  return {
    oldBitmapFrames: oldBitmapFrames === -1 ? 0 : oldBitmapFrames,
    newBitmapMs,
  };
})()`);
await sleep(300);
await shot("07b-zoom-200");

// 5. T6: Fast scrolling for 10 s (3000 px every 16 ms) while sampling memory.
const sampler = spawn("powershell.exe", ["-NoProfile", "-Command", `
  $conn = Get-NetTCPConnection -LocalPort ${port} -ErrorAction SilentlyContinue | Select-Object -First 1
  $readerPid = if ($conn) { (Get-CimInstance Win32_Process -Filter "ProcessId = $($conn.OwningProcess)").ParentProcessId } else { $null }
  for ($i = 0; $i -lt 13; $i++) {
    $procs = @()
    if ($readerPid) {
      $procs += Get-Process -Id $readerPid -ErrorAction SilentlyContinue
      $workers = Get-CimInstance Win32_Process -Filter "Name = 'pdf_worker.exe'" | Where-Object { $_.ParentProcessId -eq $readerPid }
      foreach ($w in $workers) { $procs += Get-Process -Id $w.ProcessId -ErrorAction SilentlyContinue }
    }
    if (-not $procs) { $procs = Get-Process pdf-reader, pdf_worker -ErrorAction SilentlyContinue }
    $m = ($procs | Measure-Object PrivateMemorySize64 -Sum).Sum
    $w = ($procs | Where-Object { $_.ProcessName -eq 'pdf_worker' } | Measure-Object PrivateMemorySize64 -Sum).Sum
    "{0} {1}" -f [math]::Round($m / 1MB), [math]::Round($w / 1MB)
    Start-Sleep -Milliseconds 1000
  }`]);
let samples = "";
sampler.stdout.on("data", (d) => (samples += d));
await evaluate(`window.__perf.blank.length = 0`);
const started = Date.now();
while (Date.now() - started < 10_000) {
  await evaluate(`(() => { const s = document.querySelector('main'); if (s) s.scrollTop = (s.scrollTop + 3000) % (s.scrollHeight - s.clientHeight); })()`);
  await sleep(16);
}
await new Promise((r) => sampler.on("close", r));
const mem = samples.trim().split(/\r?\n/).map((l) => l.trim().split(/\s+/).map(Number));
results.fastScroll = {
  memoryMainPlusWorkerMB: mem.map((m) => m[0]),
  workerMB: mem.map((m) => m[1]),
  maxMainPlusWorkerMB: Math.max(...mem.map((m) => m[0])),
};

// 6. Last page, then back to the first.
await evaluate(`(() => { const s = document.querySelector('main'); if (s) s.scrollTop = s.scrollHeight; })()`);
for (let i = 0; i < 80 && !(await evaluate(`window.__ready(1000)`)); i++) await sleep(100);
results.lastPageReady = await evaluate(`window.__ready(1000)`);
results.lastPageStatus = await evaluate(`document.querySelector('footer').innerText`);
await shot("07b-last-page");
await evaluate(`(() => { const s = document.querySelector('main'); if (s) s.scrollTop = 0; })()`);
for (let i = 0; i < 80 && !(await evaluate(`window.__ready(1)`)); i++) await sleep(100);
results.backToFirstReady = await evaluate(`window.__ready(1)`);
results.backToFirstStatus = await evaluate(`document.querySelector('footer').innerText`);

// 7. T7: Document idle for 10 s, measure CPU usage (< 1%).
await sleep(1000);
results.idle10sCpuPercent = await new Promise((resolve) => {
  const p = spawn("powershell.exe", ["-NoProfile", "-Command", `
    $cores = [System.Environment]::ProcessorCount
    $conn = Get-NetTCPConnection -LocalPort ${port} -ErrorAction SilentlyContinue | Select-Object -First 1
    $readerPid = if ($conn) { (Get-CimInstance Win32_Process -Filter "ProcessId = $($conn.OwningProcess)").ParentProcessId } else { $null }
    function Get-TargetProcs {
      $procs = @()
      if ($readerPid) {
        $procs += Get-Process -Id $readerPid -ErrorAction SilentlyContinue
        $workers = Get-CimInstance Win32_Process -Filter "Name = 'pdf_worker.exe'" | Where-Object { $_.ParentProcessId -eq $readerPid }
        foreach ($w in $workers) { $procs += Get-Process -Id $w.ProcessId -ErrorAction SilentlyContinue }
      }
      if (-not $procs) { $procs = Get-Process pdf-reader, pdf_worker -ErrorAction SilentlyContinue }
      return $procs
    }
    $t1 = ((Get-TargetProcs) | Measure-Object TotalProcessorTime -Sum).Sum.TotalSeconds
    Start-Sleep -Seconds 10
    $t2 = ((Get-TargetProcs) | Measure-Object TotalProcessorTime -Sum).Sum.TotalSeconds
    $pct = [math]::Round((($t2 - $t1) / (10 * $cores)) * 100, 2)
    "$pct"
  `]);
  let out = "";
  p.stdout.on("data", (d) => (out += d));
  p.on("close", () => resolve(parseFloat(out.trim()) || 0));
});

// 8. Worker crash recovery: kill our worker, scroll to pages never rendered.
await evaluate(`(() => { const s = document.querySelector('main'); if (s) s.scrollTop = s.scrollHeight * 0.7; })()`);
await sleep(1500);
killOurWorker();
await sleep(300);
await evaluate(`(() => { const s = document.querySelector('main'); if (s) s.scrollTop = s.scrollHeight * 0.3; })()`);
await sleep(2500);
results.afterCrash = await evaluate(`(() => {
  const slots = [...document.querySelectorAll('[id^=page-]')];
  return { mounted: slots.length, failed: slots.filter((s) => s.dataset.state === 'failed').map((s) => s.id), ready: slots.filter((s) => s.dataset.state === 'ready').length };
})()`);
await shot("07b-after-crash");
const retried = await evaluate(`(async () => {
  const button = document.querySelector('[data-state=failed] button');
  if (!button) return 'no failed page';
  const slot = button.closest('[id^=page-]');
  button.click();
  for (let i = 0; i < 100 && slot.dataset.state !== 'ready'; i++) await new Promise((r) => setTimeout(r, 100));
  return slot.id + ' ' + slot.dataset.state;
})()`);
results.retryAfterCrash = retried;
await evaluate(`(() => { const s = document.querySelector('main'); if (s) s.scrollTop += 5000; })()`);
await sleep(1500);
results.renderingAfterCrash = await evaluate(`[...document.querySelectorAll('[id^=page-]')].map((s) => s.dataset.state).join(',')`);

// 9. T5: Large A0 Poster (2384 x 3370 pt, 200% zoom).
if (fs.existsSync(a0Path) && fs.existsSync(appExe)) {
  try {
    const activeBefore = await evaluate(`document.querySelector('button[role="tab"][id^="tab-"][aria-selected="true"]')?.id ?? ''`);
    execFileSync(appExe, [path.resolve(a0Path)], { stdio: "ignore" });
    for (let i = 0; i < 60; i++) {
      const activeNow = await evaluate(`document.querySelector('button[role="tab"][id^="tab-"][aria-selected="true"]')?.id ?? ''`);
      if (activeNow && activeNow !== activeBefore) break;
      await sleep(250);
    }
    await sleep(1000);

    results.a0Poster = await evaluate(`(async () => {
      const select = document.querySelector('select[aria-label="縮放比例"]') || document.querySelector('header select');
      const page1 = document.querySelector('div[role="tabpanel"]:not([hidden]) #page-1') || document.getElementById('page-1');
      const canvas = page1.querySelector('canvas');
      const initialWidth = canvas.width;
      const t0 = performance.now();
      if (select && select.value !== "200") {
        const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value').set;
        setter.call(select, "200");
        select.dispatchEvent(new Event("change", { bubbles: true }));
      }
      while (true) {
        await new Promise((r) => requestAnimationFrame(r));
        if (canvas.width > initialWidth * 1.5) {
          return { renderMs: Math.round(performance.now() - t0) };
        }
        if (performance.now() - t0 > 15000) return { renderMs: -1 };
      }
    })()`);
    results.a0Poster.memoryMB = sampleOurMemoryMB();
    await shot("07b-a0-poster");
  } catch (err) {
    results.a0PosterError = String(err);
  }
}

console.log("\n==================== MEASUREMENT RESULTS ====================");
console.log(JSON.stringify(results, null, 2));

console.log("\n==================== TARGETS SUMMARY (T1 - T7) ====================");
console.log(`T1 Open warm / cold:        ${results.openWarmMs} ms / ${results.openColdMs} ms (target <= 1000 ms)`);
console.log(`T2 Normal scroll blank:     ${results.normalScroll.visibleBlankMsMax} ms (target 0 ms)`);
console.log(`T3 Fast scroll stop ready:  ${results.fastScrollStopMs} ms (target <= 500 ms)`);
if (results.zoom) {
  console.log(`T4 Zoom old bitmap frames:  ${results.zoom.oldBitmapFrames} frame(s) (target <= 1 frame)`);
  console.log(`T4 Zoom new crisp bitmap:   ${results.zoom.newBitmapMs} ms (target <= 400 ms)`);
}
if (results.a0Poster) {
  console.log(`T5 A0 poster 200% render:   ${results.a0Poster.renderMs} ms (target <= 1500 ms)`);
  console.log(`T5 A0 poster memory:        ${results.a0Poster.memoryMB} MB (target < 1000 MB)`);
}
console.log(`T6 Fast scroll max memory:  ${results.fastScroll.maxMainPlusWorkerMB} MB (target < 1000 MB)`);
console.log(`T7 Idle 10s CPU usage:      ${results.idle10sCpuPercent}% (target < 1%)`);
console.log("===================================================================\n");

ws.close();
