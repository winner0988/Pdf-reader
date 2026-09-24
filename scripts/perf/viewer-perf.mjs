// Development-only measurement for MVP-07 (docs/architecture/rendering.md#端對端量測). Never
// shipped. It drives a release build that the developer started by hand with WebView2 remote
// debugging on a local port, and talks only to that port on 127.0.0.1 (Chrome DevTools
// Protocol). It opens nothing on the network.
//
//   python tests/corpus/generate.py --large
//   cargo build --release -p pdf_worker && pnpm tauri build --no-bundle
//   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9240 //     target/release/pdf-reader.exe tests/corpus/large/output/large-1000-pages.pdf
//   node scripts/perf/viewer-perf.mjs 9240 <folder for screenshots>
//
// Measures: open to first page (warm, and with the worker killed first), how long pages are on
// screen while still blank during normal scrolling, main process + worker memory during 10 s of
// very fast scrolling, the last page and back, and recovery after killing the worker.
import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";

const [port = "9240", outDir = "."] = process.argv.slice(2);
fs.mkdirSync(outDir, { recursive: true });
let page;
for (let i = 0; i < 120 && !page; i++) {
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    page = list.find((t) => t.type === "page");
  } catch {}
  if (!page) await new Promise((r) => setTimeout(r, 500));
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
await evaluate(`(() => {
  const perf = (window.__perf = { mounted: new Map(), visibleAt: new Map(), blank: [], visibleBlank: [] });
  // Time a page is actually on screen while still blank.
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting && !perf.visibleAt.has(e.target)) perf.visibleAt.set(e.target, performance.now());
  }, { root: document.querySelector('main') });
  const note = (el) => {
    if (!(el instanceof HTMLElement) || el.getAttribute('role') !== 'img' || !el.id.startsWith('page-')) return;
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
  window.__scroller = document.querySelector('main');
  window.__ready = (n) => document.getElementById('page-' + n)?.dataset.state === 'ready';
  return true;
})()`);

const results = {};

// 1. Open -> first page, warm worker and cold worker (killed first).
async function timeOpen() {
  return evaluate(`(async () => {
    const before = document.querySelector('footer').innerText;
    const t0 = performance.now();
    await window.__TAURI_INTERNALS__.invoke('retry_open');
    // Wait until the reopened document's first page has been drawn.
    while (true) {
      await new Promise((r) => requestAnimationFrame(r));
      const p = document.getElementById('page-1');
      if (p && p.dataset.state === 'ready' && performance.now() - t0 > 5) {
        const canvas = p.querySelector('canvas');
        if (canvas.width > 1) return Math.round(performance.now() - t0);
      }
      if (performance.now() - t0 > 20000) return -1;
    }
  })()`);
}
// Scroll far away first so page 1 is not mounted (and thus not "ready" from before).
await evaluate(`window.__scroller.scrollTop = window.__scroller.scrollHeight / 2`);
await sleep(300);
results.openWarmMs = await timeOpen();
await evaluate(`window.__scroller.scrollTop = window.__scroller.scrollHeight / 2`);
await sleep(300);
try { execFileSync("taskkill", ["/IM", "pdf_worker.exe", "/F"], { stdio: "ignore" }); } catch {}
await sleep(300);
results.openColdMs = await timeOpen();
await shot("07b-first-page");

// 2. Normal-speed scrolling: 200 px every 100 ms (about 2 pages a second) for 10 s.
await evaluate(`window.__perf.blank.length = 0; window.__scroller.scrollTop = 0`);
await sleep(500);
await evaluate(`window.__perf.blank.length = 0; window.__perf.visibleBlank.length = 0`);
for (let i = 0; i < 100; i++) {
  await evaluate(`window.__scroller.scrollTop += 200`);
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

// 3. Fast scrolling for 10 s (3000 px every 16 ms) while sampling memory once a second.
const sampler = spawn("powershell.exe", ["-NoProfile", "-Command", `
  for ($i = 0; $i -lt 13; $i++) {
    $m = Get-Process pdf-reader, pdf_worker -ErrorAction SilentlyContinue | Measure-Object PrivateMemorySize64 -Sum
    $w = Get-Process pdf_worker -ErrorAction SilentlyContinue | Measure-Object PrivateMemorySize64 -Sum
    "{0} {1}" -f [math]::Round($m.Sum/1MB), [math]::Round($w.Sum/1MB)
    Start-Sleep -Milliseconds 1000
  }`]);
let samples = "";
sampler.stdout.on("data", (d) => (samples += d));
await evaluate(`window.__perf.blank.length = 0`);
const started = Date.now();
while (Date.now() - started < 10_000) {
  await evaluate(`(() => { const s = window.__scroller; s.scrollTop = (s.scrollTop + 3000) % (s.scrollHeight - s.clientHeight); })()`);
  await sleep(16);
}
await new Promise((r) => sampler.on("close", r));
const mem = samples.trim().split(/\r?\n/).map((l) => l.trim().split(/\s+/).map(Number));
results.fastScroll = {
  memoryMainPlusWorkerMB: mem.map((m) => m[0]),
  workerMB: mem.map((m) => m[1]),
  maxMainPlusWorkerMB: Math.max(...mem.map((m) => m[0])),
};

// 4. Last page, then back to the first.
await evaluate(`window.__scroller.scrollTop = window.__scroller.scrollHeight`);
for (let i = 0; i < 80 && !(await evaluate(`window.__ready(1000)`)); i++) await sleep(100);
results.lastPageReady = await evaluate(`window.__ready(1000)`);
results.lastPageStatus = await evaluate(`document.querySelector('footer').innerText`);
await shot("07b-last-page");
await evaluate(`window.__scroller.scrollTop = 0`);
for (let i = 0; i < 80 && !(await evaluate(`window.__ready(1)`)); i++) await sleep(100);
results.backToFirstReady = await evaluate(`window.__ready(1)`);
results.backToFirstStatus = await evaluate(`document.querySelector('footer').innerText`);

// 5. Worker crash: kill it, then scroll to pages that were never rendered.
await evaluate(`window.__scroller.scrollTop = window.__scroller.scrollHeight * 0.7`);
await sleep(1500);
try { execFileSync("taskkill", ["/IM", "pdf_worker.exe", "/F"], { stdio: "ignore" }); } catch {}
await sleep(300);
await evaluate(`window.__scroller.scrollTop = window.__scroller.scrollHeight * 0.3`);
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
await evaluate(`window.__scroller.scrollTop += 5000`);
await sleep(1500);
results.renderingAfterCrash = await evaluate(`[...document.querySelectorAll('[id^=page-]')].map((s) => s.dataset.state).join(',')`);

console.log(JSON.stringify(results, null, 2));
ws.close();
