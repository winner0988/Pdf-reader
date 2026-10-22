// Development-only performance measurement of the viewer (MVP-07, B3-09:
// "效能目標" in docs/architecture/rendering.md). Never shipped, never run on CI.
//
// It starts a release build itself, with a WebView2 profile and an app data folder of its own (the
// developer's recent files and settings are not touched), with WebView2 remote debugging on a free
// port of 127.0.0.1, and talks only to that port (Chrome DevTools Protocol). It opens nothing on the
// network. The app is single-instance: if a build with the same identifier is already running, the
// new one hands the file to it and quits, and this script stops and says so.
//
//   python tests/corpus/generate.py --large
//   cargo build --release -p pdf_worker && pnpm tauri build --no-bundle
//   node scripts/perf/viewer-perf.mjs [--app <pdf-reader.exe>] [--out <folder>]
//
// It prints the targets T1-T7 next to what it measured, writes them to <folder>/results.json with
// screenshots of the page, and checks recovery after the worker is killed, as MVP-07 did.
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
const large = path.join(ROOT, "tests", "corpus", "large", "output", "large-1000-pages.pdf");
const poster = path.join(ROOT, "tests", "corpus", "large", "output", "large-a0-poster.pdf");
for (const file of [app, large, poster]) {
  if (!fs.existsSync(file)) {
    console.error(`Missing ${file}: see the commands at the top of this script.`);
    process.exit(1);
  }
}
fs.mkdirSync(outDir, { recursive: true });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const pct = (values, p) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted.length ? Math.round(sorted[Math.min(sorted.length - 1, Math.round((sorted.length - 1) * p))]) : NaN;
};

async function freePort() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

// ---------------------------------------------------------------- the app and its processes

const port = await freePort();
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "pdf-reader-perf-"));
const data = fs.mkdtempSync(path.join(os.tmpdir(), "pdf-reader-perf-data-"));
const env = {
  ...process.env,
  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  WEBVIEW2_USER_DATA_FOLDER: profile,
  PDF_READER_DATA_DIR: data,
};
const launched = performance.now();
const child = spawn(app, [large], { env, stdio: "ignore" });
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

/** Private memory in MB: the main process with its workers, and all of it with WebView2. */
const memoryScript = `
  $mine = $tree | Where-Object { $_.Name -in 'pdf-reader.exe', 'pdf_worker.exe' }
  $main = ($mine | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  $every = ($tree | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).PrivateMemorySize64 } | Measure-Object -Sum).Sum
  "{0} {1}" -f [math]::Round($main / 1MB), [math]::Round($every / 1MB)
`;
function memoryMB() {
  const [main, all] = powershell(treeScript + memoryScript).split(/\s+/).map(Number);
  return { main, all };
}

function killWorkers() {
  powershell(`${treeScript}
    $tree | Where-Object { $_.Name -eq 'pdf_worker.exe' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }`);
}

/** CPU used by the app, its workers and WebView2 over `seconds`, as a share of all cores. */
function cpuPercent(seconds) {
  return Number(
    powershell(`${treeScript}
      $cpu = { ($tree | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).TotalProcessorTime.TotalSeconds } | Measure-Object -Sum).Sum }
      $t1 = & $cpu
      Start-Sleep -Seconds ${seconds}
      $t2 = & $cpu
      [math]::Round(($t2 - $t1) / (${seconds} * [Environment]::ProcessorCount) * 100, 2)`),
  );
}

/** Opens `file` in the running app the way a double-click does: a second start hands it over. */
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
const shot = async (name) => {
  const reply = await send("Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(path.join(outDir, `${name}.png`), Buffer.from(reply.result.data, "base64"));
};
/** Polls `expression` in the page until it is true; milliseconds it took, or -1. */
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

// Helpers in the page. A page slot is the `role="img"` element of a page (`data-doc` tells it from
// a thumbnail), named by strings.canvas.page.
const helpers = `(() => {
  if (window.__perf) return true;
  // Every tab has its own page area; the shown one is laid out.
  const scroller = () => [...document.querySelectorAll('main[data-region="canvas"]')].find((m) => m.getClientRects().length > 0);
  const slots = () => [...(scroller()?.querySelectorAll('[role="img"][data-doc]') ?? [])].filter((el) => el.getClientRects().length > 0);
  const drawn = (el) => el?.dataset.state === 'ready' && (el.querySelector('canvas')?.width ?? 0) > 1;
  const visible = () => {
    const box = scroller().getBoundingClientRect();
    return slots().filter((el) => { const r = el.getBoundingClientRect(); return r.bottom > box.top && r.top < box.bottom; });
  };
  Object.assign(window, {
    __scroller: scroller,
    __slot: (n) => slots().find((el) => el.getAttribute('aria-label') === '第 ' + n + ' 頁'),
    __ready: (n) => drawn(window.__slot(n)),
    __doc: () => slots()[0]?.dataset.doc ?? '',
    __visible: visible,
    __allVisibleDrawn: () => { const v = visible(); return v.length > 0 && v.every(drawn); },
    __zoom: (value) => {
      // The shown tab's zoom box (strings.toolbar.zoomLevel).
      const select = [...document.querySelectorAll('select[aria-label="縮放比例"]')].find((el) => el.getClientRects().length > 0);
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set.call(select, String(value));
      select.dispatchEvent(new Event('change', { bubbles: true }));
    },
    __frames: () => new Promise((resolve) => requestAnimationFrame(() => resolve())),
  });
  // How long each page is on screen while still blank (MVP-07's measure).
  const perf = (window.__perf = { mounted: new Map(), shownAt: new Map(), blankShown: [] });
  const io = new IntersectionObserver((entries) => {
    for (const e of entries) if (e.isIntersecting && !perf.shownAt.has(e.target)) perf.shownAt.set(e.target, performance.now());
  }, { root: scroller() });
  const note = (el) => {
    if (!(el instanceof HTMLElement) || el.getAttribute('role') !== 'img' || !el.dataset.doc) return;
    if (el.dataset.state === 'loading' && !perf.mounted.has(el)) { perf.mounted.set(el, performance.now()); io.observe(el); }
    if (el.dataset.state === 'ready' && perf.mounted.has(el)) {
      perf.blankShown.push(perf.shownAt.has(el) ? performance.now() - perf.shownAt.get(el) : 0);
      perf.mounted.delete(el);
      io.unobserve(el);
    }
  };
  new MutationObserver((records) => {
    for (const r of records) {
      if (r.type === 'attributes') note(r.target);
      r.addedNodes.forEach((n) => { note(n); n.querySelectorAll?.('[role="img"]').forEach(note); });
    }
  }).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['data-state'] });
  return true;
})()`;

const results = {};
/** Says on stderr what is being measured, so a run that stalls shows where. */
const step = (name) => console.error(`${new Date().toISOString().slice(11, 19)} ${name}`);
// A stalled run still closes the app and removes its folders.
setTimeout(() => fail("Stopped after 10 minutes."), 600_000).unref();

step("T1");
// T1: start the app with the 1000-page file until its first page is drawn; then open the same file
// again in the running app (a second tab).
const startedMs = await (async () => {
  while (performance.now() - launched < 60_000) {
    try {
      if (await evaluate(`document.readyState === 'complete' && ${helpers} && window.__ready(1)`)) {
        return Math.round(performance.now() - launched);
      }
    } catch {}
    await sleep(20);
  }
  return -1;
})();
if (startedMs < 0) fail("The first page was never drawn.");
results.startToFirstPageMs = startedMs;
results.window = await evaluate(`({
  width: innerWidth,
  height: innerHeight,
  devicePixelRatio,
  page1Raster: window.__slot(1).querySelector('canvas').width + ' x ' + window.__slot(1).querySelector('canvas').height,
})`);
await shot("t1-first-page");

const firstDoc = await evaluate(`window.__doc()`);
const t0 = performance.now();
openInRunningApp(large);
results.openInRunningAppMs = await until(`window.__doc() !== ${JSON.stringify(firstDoc)} && window.__ready(1)`);
if (results.openInRunningAppMs >= 0) results.openInRunningAppMs = Math.round(performance.now() - t0);
await pressCtrl("w", 87);
await until(`window.__doc() === ${JSON.stringify(firstDoc)} && window.__ready(1)`);

step("T2");
// T2: normal-speed scrolling, 200 px every 100 ms (about two pages a second) for 10 s.
await evaluate(`window.__scroller().scrollTop = 0`);
await until(`window.__allVisibleDrawn()`);
await evaluate(`window.__perf.blankShown.length = 0`);
for (let i = 0; i < 100; i++) {
  await evaluate(`window.__scroller().scrollTop += 200`);
  await sleep(100);
}
await until(`window.__allVisibleDrawn()`);
const shown = await evaluate(`window.__perf.blankShown`);
results.normalScroll = {
  pages: shown.length,
  blankShownMsP50: pct(shown, 0.5),
  blankShownMsP95: pct(shown, 0.95),
  blankShownMsMax: pct(shown, 1),
};

step("T3");
// T3: very fast scrolling (3000 px every 16 ms for 1 s), then stop: until every page on screen is drawn.
results.afterFastScrollMs = [];
for (const start of [0.2, 0.4, 0.6, 0.8]) {
  results.afterFastScrollMs.push(
    await evaluate(`(async () => {
      const s = window.__scroller();
      s.scrollTop = Math.round(s.scrollHeight * ${start});
      const end = performance.now() + 1000;
      while (performance.now() < end) {
        s.scrollTop += 3000;
        await new Promise((r) => setTimeout(r, 16));
      }
      const t0 = performance.now();
      while (!window.__allVisibleDrawn()) {
        if (performance.now() - t0 > 20000) return -1;
        await window.__frames();
      }
      return Math.round(performance.now() - t0);
    })()`),
  );
}

// T4: zoom from 100% to 200% and to 400% at page 1: frames with a blank page on screen, and time
// until the part on screen is drawn at the new zoom (the old drawing is stretched until then).
// The page says so with `data-sharp` (B3-09): the whole page at the zoom, or, zoomed far in, the
// part on screen over a lower-resolution page. Builds before B3-09 have no such mark: for them,
// sharp is a newly drawn canvas of another size, as the baseline was measured.
async function zoomTo(from, to) {
  await evaluate(`window.__zoom(${JSON.stringify(from)})`);
  await sleep(300);
  await evaluate(`window.__scroller().scrollTop = 0`);
  await until(`window.__allVisibleDrawn()`);
  await sleep(500);
  return evaluate(`(async () => {
    const marked = window.__slot(1).hasAttribute('data-sharp');
    const width = window.__slot(1).querySelector('canvas').width;
    const t0 = performance.now();
    window.__zoom(${JSON.stringify(to)});
    let blankFrames = 0;
    let zoomed = false;
    while (performance.now() - t0 < 20000) {
      await window.__frames();
      const slot = window.__slot(1);
      const canvas = slot?.querySelector('canvas');
      if (!canvas || canvas.width <= 1) blankFrames++;
      if (!marked) {
        if (canvas && slot.dataset.state === 'ready' && canvas.width > 1 && canvas.width !== width) {
          return { sharpMs: Math.round(performance.now() - t0), blankFrames, drawn: 'whole ' + canvas.width + ' x ' + canvas.height };
        }
        continue;
      }
      // Not sharp while the zoom settles; sharp again once drawn at the new one.
      if (slot && !slot.hasAttribute('data-sharp')) zoomed = true;
      else if (zoomed && slot) {
        const detail = slot.querySelector('canvas[data-detail]');
        return {
          sharpMs: Math.round(performance.now() - t0),
          blankFrames,
          drawn: detail && !detail.hidden ? 'part ' + detail.width + ' x ' + detail.height : 'whole ' + canvas.width + ' x ' + canvas.height,
        };
      }
    }
    return { sharpMs: -1, blankFrames };
  })()`);
}
step("T4");
results.zoom200 = await zoomTo(100, 200);
await shot("t4-zoom-200");
results.zoom400 = await zoomTo(100, 400);
await evaluate(`window.__zoom('fitWidth')`);
await sleep(500);

step("T6");
// T6: very fast scrolling for 10 s while sampling memory once a second.
const sampler = spawn("powershell.exe", [
  "-NoProfile",
  "-Command",
  `${treeScript}
  for ($i = 0; $i -lt 10; $i++) {
    ${memoryScript}
    Start-Sleep -Milliseconds 700
  }`,
]);
let samples = "";
sampler.stdout.on("data", (chunk) => (samples += chunk));
const sampled = new Promise((resolve) => sampler.on("close", resolve));
const scrolling = Date.now();
while (Date.now() - scrolling < 10_000) {
  await evaluate(`(() => { const s = window.__scroller(); s.scrollTop = (s.scrollTop + 3000) % (s.scrollHeight - s.clientHeight); })()`);
  await sleep(16);
}
await sampled;
const memory = samples
  .trim()
  .split(/\r?\n/)
  .map((line) => line.trim().split(/\s+/).map(Number));
results.fastScrollMemoryMB = {
  mainAndWorkers: memory.map((m) => m[0]),
  withWebView2: memory.map((m) => m[1]),
  maxMainAndWorkers: Math.max(...memory.map((m) => m[0])),
};

step("last page");
// The last page and back to the first.
await evaluate(`window.__scroller().scrollTop = window.__scroller().scrollHeight`);
results.lastPageMs = await until(`window.__ready(1000)`, 10_000);
await shot("last-page");
await evaluate(`window.__scroller().scrollTop = 0`);
results.backToFirstMs = await until(`window.__ready(1)`, 10_000);

step("T7");
// T7: the document left alone for 10 s.
await sleep(2000);
results.idleCpuPercent = cpuPercent(10);
results.idleMemoryMB = memoryMB();
// Where the time of a large page goes: the worker and the main process (until the bytes are in
// the page), then drawing them. Nothing else is being drawn now, and the scales were not asked
// for before, so nothing comes from the cache.
results.largePageBreakdown = await evaluate(`(async () => {
  const doc = Number(window.__doc());
  const out = [];
  // The last, a part of the page as the view draws it zoomed far in (B3-09).
  const part = { x: 200, y: 300, width: 2700, height: 1800 };
  for (const [scale, region] of [[2.6661], [5.3331], [5.3332, part]]) {
    const t0 = performance.now();
    // Builds before B3-09 refuse a region.
    const bytes = await window.__TAURI_INTERNALS__
      .invoke('render_page', { args: { request: 4000000 + Math.round(scale * 10000), doc, pageIndex: 0, scale, rotation: 'none', region } })
      .catch(() => null);
    if (!bytes) {
      out.push({ scale, region: part.width + ' x ' + part.height, error: 'refused' });
      continue;
    }
    const t1 = performance.now();
    const view = new DataView(bytes);
    const width = view.getUint32(8, true);
    const height = view.getUint32(12, true);
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    canvas.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(bytes, 16, width * height * 4), width, height), 0, 0);
    const t2 = performance.now();
    out.push({ scale, region: region ? width + ' x ' + height : 'whole page', megabytes: Math.round(bytes.byteLength / 1048576), renderAndTransferMs: Math.round(t1 - t0), drawMs: Math.round(t2 - t1) });
  }
  return out;
})()`);

step("recovery");
// Recovery: kill the worker, scroll to pages never drawn; failed pages offer a retry that works.
await evaluate(`window.__scroller().scrollTop = window.__scroller().scrollHeight * 0.7`);
await sleep(1500);
killWorkers();
await sleep(300);
await evaluate(`window.__scroller().scrollTop = window.__scroller().scrollHeight * 0.3`);
await sleep(2500);
results.afterCrash = await evaluate(`(() => {
  const v = window.__visible();
  return { shown: v.length, failed: v.filter((s) => s.dataset.state === 'failed').length, drawn: v.filter((s) => s.dataset.state === 'ready').length };
})()`);
results.retryAfterCrash = await evaluate(`(async () => {
  const button = window.__scroller().querySelector('[data-state=failed] button');
  if (!button) return 'no failed page';
  const slot = button.closest('[role="img"]');
  button.click();
  for (let i = 0; i < 100 && slot.dataset.state !== 'ready'; i++) await new Promise((r) => setTimeout(r, 100));
  return slot.getAttribute('aria-label') + ': ' + slot.dataset.state;
})()`);
await evaluate(`window.__scroller().scrollTop += 5000`);
results.drawingAfterCrashMs = await until(`window.__allVisibleDrawn()`, 10_000);

step("T5");
// T5: an A0 poster (2384 x 3370 pt) at 200%.
const beforePoster = await evaluate(`window.__doc()`);
openInRunningApp(poster);
results.posterOpenMs = await until(`window.__doc() !== ${JSON.stringify(beforePoster)} && window.__ready(1)`);
results.poster200 = await zoomTo("fitWidth", 200).catch((error) => ({ error: String(error) }));
results.posterMemoryMB = memoryMB();
await shot("t5-poster-200");

ws.close();
cleanUp();

// ---------------------------------------------------------------- report

const targets = [
  ["T1 start to first page (1000 pages, 197 MB)", `${results.startToFirstPageMs} ms`, "<= 1500 ms", results.startToFirstPageMs <= 1500],
  ["T1 open in the running app", `${results.openInRunningAppMs} ms`, "<= 1000 ms", results.openInRunningAppMs >= 0 && results.openInRunningAppMs <= 1000],
  ["T2 normal scrolling: blank on screen p95 / max", `${results.normalScroll.blankShownMsP95} / ${results.normalScroll.blankShownMsMax} ms`, "p95 0 ms, max <= 100 ms", results.normalScroll.blankShownMsP95 === 0 && results.normalScroll.blankShownMsMax <= 100],
  ["T3 stop after fast scrolling: all drawn (max)", `${Math.max(...results.afterFastScrollMs)} ms`, "<= 500 ms", results.afterFastScrollMs.every((ms) => ms >= 0 && ms <= 500)],
  ["T4 zoom 100% -> 200%: blank frames / sharp", `${results.zoom200.blankFrames} / ${results.zoom200.sharpMs} ms`, "0 / <= 500 ms", results.zoom200.blankFrames === 0 && results.zoom200.sharpMs >= 0 && results.zoom200.sharpMs <= 500],
  ["T4 zoom 100% -> 400%: blank frames / sharp", `${results.zoom400.blankFrames} / ${results.zoom400.sharpMs} ms`, "0 / <= 1500 ms", results.zoom400.blankFrames === 0 && results.zoom400.sharpMs >= 0 && results.zoom400.sharpMs <= 1500],
  ["T5 A0 poster at 200%: sharp", `${results.poster200.sharpMs} ms`, "<= 1500 ms", results.poster200.sharpMs >= 0 && results.poster200.sharpMs <= 1500],
  ["T5 A0 poster: main process + workers", `${results.posterMemoryMB.main} MB`, "< 1024 MB", results.posterMemoryMB.main < 1024],
  ["T6 fast scrolling 10 s: main process + workers (max)", `${results.fastScrollMemoryMB.maxMainAndWorkers} MB`, "< 1024 MB", results.fastScrollMemoryMB.maxMainAndWorkers < 1024],
  ["T7 idle 10 s: CPU of the app, workers and WebView2", `${results.idleCpuPercent} %`, "< 1 %", results.idleCpuPercent < 1],
];
results.targets = targets.map(([name, value, target, met]) => ({ name, value, target, met }));
fs.writeFileSync(path.join(outDir, "results.json"), JSON.stringify(results, null, 2));
console.log(JSON.stringify(results, null, 2));
console.log("");
for (const [name, value, target, met] of targets) {
  console.log(`${met ? "ok  " : "MISS"} ${name}: ${value} (target ${target})`);
}
