// Starts the real app and drives its WebView over the Chrome DevTools Protocol (QA-02,
// docs/architecture/e2e.md). WebView2 reads its launch settings from environment variables:
// each test gets a remote debugging port on 127.0.0.1 and a fresh, temporary WebView2 profile,
// so it has its own browser process and nothing carries over between tests. The app's own data
// (the recent files list, #73) goes to a temporary folder too, never to the user's.

import { execFile, execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { get } from "node:http";
import { createServer, type AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";
import { promisify } from "node:util";

import { chromium, test as base, type Browser, type CDPSession, type Page, type Request } from "@playwright/test";

export const ROOT = path.resolve(import.meta.dirname, "../..");

/** The app under test: the release build next to its pdf_worker.exe (see docs/architecture/e2e.md). */
const APP = process.env.E2E_APP ?? path.join(ROOT, "target", "release", "pdf-reader.exe");

/** A file of the test corpus (tests/corpus, QA-01). No other PDFs are ever opened. */
export const corpus = (file: string) => path.join(ROOT, "tests", "corpus", file);

const STARTUP_TIMEOUT_MS = 30_000;

/**
 * WebView2 Runtime 150 and later ignore WEBVIEW2_* environment variables and per-user policy in
 * elevated processes; only machine policy still adds browser arguments there. GitHub's Windows
 * runners run elevated, so on CI (only: it changes machine-wide settings) the debugging port is
 * also set as that policy, for this executable, while the app starts. Like the environment
 * variable, it adds to the arguments the app itself passes (src-tauri/tauri.conf.json, such as
 * those that stop WebView2 connecting on its own, #121): the app runs as it ships.
 */
const MACHINE_POLICY = "HKLM\\Software\\Policies\\Microsoft\\Edge\\WebView2\\AdditionalBrowserArguments";

function setDebuggingPolicy(browserArguments: string | null) {
  if (!process.env.CI) return;
  const name = path.basename(APP);
  const args =
    browserArguments === null
      ? ["delete", MACHINE_POLICY, "/v", name, "/f"]
      : ["add", MACHINE_POLICY, "/v", name, "/t", "REG_SZ", "/d", browserArguments, "/f"];
  execFileSync("reg", args, { stdio: "ignore" });
}

async function freePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

type Running = {
  child: ChildProcess;
  log: string[];
  profile: string;
  data: string;
  browser?: Browser;
  /** The page `launch` returned: the one the test drives. */
  page?: Page;
  /** When the app was started, when its WebView answered, and when its page appeared (`Date.now()`). */
  launched: number;
  connected?: number;
  ready?: number;
  /** What the app and the computer were doing while its page was slow to appear (#133). */
  slowStart?: Promise<string>;
  /** The page's calls to the main process (`invoke`) still waiting for an answer, with when each was made. */
  pendingIpc: Map<Request, { command: string; at: number }>;
  /** How many calls of each command were answered (or failed). */
  answeredIpc: Map<string, number>;
  /** The page's renderer counters (CDP `Performance`), from when the test got the page. */
  performance?: CDPSession;
};

/**
 * Runs in every document the page loads: notes each time its timers or its animation frames
 * stopped for more than a quarter of a second (when, in ms since the document started, and for
 * how long), for `summary` (#133). The test's own record: the app never reads it.
 */
function heartbeat() {
  const page = window as unknown as { __e2eStalls?: { timer: string[]; frame: string[] } };
  if (page.__e2eStalls) return;
  const stalls: { timer: string[]; frame: string[] } = { timer: [], frame: [] };
  page.__e2eStalls = stalls;
  const note = (list: string[], last: number, now: number) => {
    if (now - last > 250 && list.length < 50) list.push(`${Math.round(last)}+${Math.round(now - last)}`);
  };
  let timerLast = performance.now();
  setInterval(() => {
    const now = performance.now();
    note(stalls.timer, timerLast, now);
    timerLast = now;
  }, 100);
  let frameLast = performance.now();
  const frame = (now: number) => {
    note(stalls.frame, frameLast, now);
    frameLast = now;
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}

/** Where the page's `invoke` calls go: Tauri's IPC, one URL path per command. */
const IPC_ORIGIN = "http://ipc.localhost/";

/** Follows the page's calls to the main process, for `summary`. */
function followIpc(page: Page, running: Running) {
  page.on("request", (request) => {
    if (request.url().startsWith(IPC_ORIGIN)) {
      running.pendingIpc.set(request, { command: new URL(request.url()).pathname.slice(1), at: Date.now() });
    }
  });
  const answered = (request: Request) => {
    const call = running.pendingIpc.get(request);
    if (!call) return;
    running.pendingIpc.delete(request);
    running.answeredIpc.set(call.command, (running.answeredIpc.get(call.command) ?? 0) + 1);
  };
  page.on("requestfinished", answered);
  page.on("requestfailed", answered);
}

/** How much of the end of a failed test's app log also goes to the test's output (CI's log). */
const LOG_TAIL_CHARS = 3_000;

/** `promise`'s outcome, or a note that there was none within `ms`: a stuck page must not stop the report. */
async function within<T>(promise: Promise<T>, ms: number): Promise<T | string> {
  let timer: NodeJS.Timeout | undefined;
  return Promise.race([
    promise,
    new Promise<string>((resolve) => {
      timer = setTimeout(() => resolve(`no answer within ${ms / 1_000} s`), ms);
    }),
  ])
    .catch((error: unknown) => `failed: ${String(error).split("\n")[0]}`)
    .finally(() => clearTimeout(timer));
}

/**
 * A failed test's app, for the test's output and so for CI's log: how its start went, what its
 * page shows, and the end of its log. Enough to tell a document still opening from an error or a
 * page that never laid out, without downloading the uploaded files (#133).
 *
 * The test's locators run in Playwright's own script world in the page, `evaluate` in the page's.
 * On CI they have disagreed: the page's text showed the document while the test's locators found
 * nothing in it, not even the status bar, and the pages never finished rendering (#133). So this
 * also says which pages the WebView has, what the test's locators see, and whether the page's own
 * timers, frames and calls to the main process still answer.
 */
async function summary(running: Running): Promise<string> {
  const started =
    running.connected === undefined
      ? "its WebView never answered"
      : `its WebView answered ${running.connected - running.launched} ms after launch`;
  const state = running.child.exitCode === null ? "still running" : `exited with ${running.child.exitCode}`;
  const pages = running.browser?.contexts().flatMap((context) => context.pages()) ?? [];
  const page = running.page ?? pages[0];
  const appeared =
    running.ready === undefined
      ? "its page never appeared"
      : `its page appeared ${running.ready - running.launched} ms after launch`;
  const lines = [
    `--- app: ${started}; ${appeared}; ${Date.now() - running.launched} ms after launch it is ${state} (launched at ${running.launched}) ---`,
    ...(running.slowStart ? [`while its page was slow to appear: ${await running.slowStart}`] : []),
    `pages: ${JSON.stringify(pages.map((each) => `${each.url()}${each === page ? " (the test's)" : ""}`))}`,
    // Whether the page asked for its pages to be rendered, and whether the main process answered.
    `its calls to the main process: ${JSON.stringify({
      answered: Object.fromEntries(running.answeredIpc),
      waiting: [...running.pendingIpc.values()].map(({ command, at }) => `${command} for ${Date.now() - at} ms`),
    })}`,
  ];
  if (page) {
    const shown = await within(
      page.evaluate(() => ({
        url: location.href,
        readyState: document.readyState,
        visibility: document.visibilityState,
        focused: document.hasFocus(),
        viewport: `${window.innerWidth}x${window.innerHeight}`,
        // When this document started, since the Unix epoch: a page that loaded again shows here.
        started: Math.round(performance.timeOrigin),
        text: document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 1_000),
        pageSlots: Array.from(document.querySelectorAll("[role=img][data-state]"), (slot) =>
          `${slot.getAttribute("aria-label")} ${slot.getAttribute("data-state")}`,
        ).slice(0, 5),
        // Anything that would hide the status bar or a page from the test's role locators.
        hiding: Array.from(document.querySelectorAll("[aria-hidden=true], [inert]"))
          .filter((element) => element.querySelector("footer, [role=img]"))
          .map((element) => element.outerHTML.slice(0, 120)),
      })),
      5_000,
    );
    lines.push(`page: ${JSON.stringify(shown)}`);
    // When, in ms since the document started, its own files arrived, it was parsed and loaded,
    // and each call to the main process was made and answered (the page's resource timing): a
    // page that waited on something shows where.
    const timeline = await within(
      page.evaluate(() => {
        const at = (ms: number) => Math.round(ms);
        const [navigation] = performance.getEntriesByType("navigation") as PerformanceNavigationTiming[];
        const resources = performance.getEntriesByType("resource") as PerformanceResourceTiming[];
        const named = (entry: PerformanceResourceTiming) => new URL(entry.name).pathname.split("/").pop() || entry.name;
        return {
          parsed: navigation ? at(navigation.domContentLoadedEventEnd) : null,
          loaded: navigation ? at(navigation.loadEventEnd) : null,
          files: resources
            .filter((entry) => entry.name.startsWith("http://tauri.localhost/"))
            .map((entry) => `${named(entry)} ${at(entry.startTime)}-${at(entry.responseEnd)}`),
          calls: resources
            .filter((entry) => entry.name.startsWith("http://ipc.localhost/"))
            .slice(0, 20)
            .map((entry) => `${named(entry)} ${at(entry.startTime)}-${at(entry.responseEnd)}`),
        };
      }),
      3_000,
    );
    lines.push(`its timeline: ${JSON.stringify(timeline)}`);
    const timer = await within(page.evaluate(() => new Promise((resolve) => setTimeout(() => resolve("fired"), 10))), 3_000);
    const frame = await within(page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve("drawn")))), 3_000);
    // The canvas's width over ten frames: a fit-width page re-renders whenever it changes.
    const widths = await within(
      page.evaluate(
        () =>
          new Promise<number[]>((resolve) => {
            const seen: number[] = [];
            const sample = () => {
              seen.push(document.querySelector("main")?.clientWidth ?? -1);
              if (seen.length < 10) requestAnimationFrame(sample);
              else resolve([...new Set(seen)]);
            };
            requestAnimationFrame(sample);
          }),
      ),
      3_000,
    );
    // A read-only call, as the page's own `invoke` makes it.
    const ipc = await within(
      page.evaluate(() =>
        (window as unknown as { __TAURI_INTERNALS__: { invoke(command: string): Promise<unknown> } }).__TAURI_INTERNALS__
          .invoke("get_settings")
          .then(() => "answered"),
      ),
      3_000,
    );
    lines.push(`its script: ${JSON.stringify({ timer, frame, widths, ipc })}`);
    // Its main thread since the test got the page: the stalls the page saw, and the time spent in
    // tasks of each kind (ms of wall time; a task waiting on something counts too).
    const stalls = await within(
      page.evaluate(() => (window as unknown as { __e2eStalls?: unknown }).__e2eStalls ?? "none recorded"),
      3_000,
    );
    const busy = running.performance
      ? await within(
          running.performance.send("Performance.getMetrics").then(({ metrics }) =>
            Object.fromEntries(
              metrics
                .filter(({ name }) => /^(Task|Script|Layout|RecalcStyle|V8Compile)Duration$|^(Layout|RecalcStyle)Count$/.test(name))
                .map(({ name, value }) => [name, name.endsWith("Duration") ? Math.round(value * 1_000) : value]),
            ),
          ),
          3_000,
        )
      : "not measured";
    lines.push(`its main thread: ${JSON.stringify({ stalls, busy })}`);
    // The first page slot as React has it, read from React's own fields on its element (here
    // only, on failure): whether it waits for the scale to settle, and whether it is still the
    // same element half a second later (a slot that keeps mounting anew never asks).
    const slot = await within(
      page.evaluate(async () => {
        type Fiber = { memoizedProps?: Record<string, unknown> | null; return?: Fiber | null };
        const element = document.querySelector("[role=img][data-state]");
        if (!element) return "no page slot";
        const key = Object.keys(element).find((name) => name.startsWith("__reactFiber$"));
        let fiber = key ? (element as unknown as Record<string, Fiber | undefined>)[key] : undefined;
        let props: Record<string, unknown> | null = null;
        for (let depth = 0; fiber && depth < 5 && !props; depth++, fiber = fiber.return ?? undefined) {
          if (fiber.memoizedProps && "paused" in fiber.memoizedProps) props = fiber.memoizedProps;
        }
        await new Promise((resolve) => setTimeout(resolve, 500));
        return {
          paused: props?.paused,
          scale: props?.scale,
          requestDelayMs: props?.requestDelayMs,
          doc: props?.doc,
          renderer: props ? props.renderer !== undefined : "no props found",
          sameElement: element.isConnected,
        };
      }),
      3_000,
    );
    lines.push(`its first page slot: ${JSON.stringify(slot)}`);
    const statusBar = await within(page.getByRole("contentinfo").count(), 3_000);
    const text = await within(
      page
        .locator("body")
        .innerText({ timeout: 3_000 })
        .then((body) => body.replace(/\s+/g, " ").trim().slice(0, 200)),
      4_000,
    );
    lines.push(`the test's locators: ${JSON.stringify({ statusBar, text })}`);
  } else {
    lines.push("page: none");
  }
  lines.push("end of its log:", running.log.join("").slice(-LOG_TAIL_CHARS));
  return lines.join("\n");
}

/** Each page's app data folder (`PDF_READER_DATA_DIR`). */
const dataDirs = new WeakMap<Page, string>();

/** Where the app behind `page` keeps its own data, such as `recent.json` (#73). */
export function dataDir(page: Page): string {
  const dir = dataDirs.get(page);
  if (!dir) throw new Error("not a page from launch()");
  return dir;
}

/** The app process behind each page. */
const processes = new WeakMap<Page, number>();

/** The app behind each page, for `quit`. */
const apps = new WeakMap<Page, Running>();

/** The id of the app process behind `page`, e.g. to find its windows with UI Automation. */
export function appProcessId(page: Page): number {
  const id = processes.get(page);
  if (id === undefined) throw new Error("not a page from launch()");
  return id;
}

/** Whether something answers on the WebView's debugging port (loopback only). */
function listening(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const request = get({ host: "127.0.0.1", port, path: "/json/version", timeout: 1_000 }, (response) => {
      response.resume();
      resolve(response.statusCode === 200);
    });
    request.on("error", () => resolve(false));
    request.on("timeout", () => {
      request.destroy();
      resolve(false);
    });
  });
}

/** Waits until the WebView answers on its debugging port, then connects to it. */
async function connect(port: number, running: Running): Promise<Browser> {
  const endpoint = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (running.child.exitCode !== null) {
      throw new Error(`the app exited with ${running.child.exitCode}:\n${running.log.join("")}`);
    }
    if (await listening(port)) return chromium.connectOverCDP(endpoint);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  // Its browser process's command line says whether WebView2 took the test's arguments: shown in
  // CI's log, not only in the uploaded files.
  const browserProcess = running.child.pid === undefined ? "" : browserCommandLine(running.child.pid);
  throw new Error(
    `the app's WebView did not start within ${STARTUP_TIMEOUT_MS} ms; its WebView2 browser process:\n` +
      (browserProcess || "(none)"),
  );
}

/**
 * How long WebView2 may take to load the app's page. In the fresh profile each test gets, CI has
 * seen it take 17 to 35 s: the app's HTML arrived that late, and then everything was fast (#133).
 */
const PAGE_TIMEOUT_MS = 60_000;
/** A page slower than this to appear is noted in the test's output, with what was going on. */
const SLOW_PAGE_MS = 5_000;

/**
 * Waits until the app's page has rendered (React has put something in `#root`), so that a test's
 * own timeouts measure the app, not WebView2 starting up. When the page is slow to appear, notes
 * whether the app's window answers (`IsHungAppWindow`: is the app's main thread stuck?), how much
 * processor time the app and its WebView2 processes have used, and what the computer is busy with.
 */
async function waitForApp(page: Page, running: Running) {
  const pid = running.child.pid;
  const timer =
    pid === undefined
      ? undefined
      : setTimeout(() => {
          running.slowStart = startSnapshot(pid);
        }, SLOW_PAGE_MS);
  try {
    await page.locator("#root > *").first().waitFor({ state: "attached", timeout: PAGE_TIMEOUT_MS });
  } finally {
    clearTimeout(timer);
  }
  running.ready = Date.now();
  if (running.slowStart) {
    console.log(
      `--- app: its page appeared ${running.ready - running.launched} ms after launch; meanwhile ${await running.slowStart}`,
    );
  }
}

/** The app's window and processes, and the computer, now: see `waitForApp`. */
async function startSnapshot(pid: number): Promise<string> {
  const script = [
    `Add-Type -Namespace E2e -Name Hung -MemberDefinition '[DllImport("user32.dll")] public static extern bool IsHungAppWindow(System.IntPtr window);'`,
    `$window = (Get-Process -Id ${pid}).MainWindowHandle`,
    "$state = if ($window -eq [System.IntPtr]::Zero) { 'no window yet' } elseif ([E2e.Hung]::IsHungAppWindow($window)) { 'the window does not answer' } else { 'the window answers' }",
    "$all = Get-CimInstance Win32_Process",
    `$tree = @(${pid})`,
    "do { $more = $all | Where-Object { $tree -contains $_.ParentProcessId -and $tree -notcontains $_.ProcessId }; $tree += $more.ProcessId } while ($more)",
    String.raw`$processes = $all | Where-Object { $tree -contains $_.ProcessId } | ForEach-Object { $cpu = (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).CPU; $type = if ($_.CommandLine -match '--type=([a-z-]+)') { $Matches[1] } else { 'main' }; "$($_.Name) ($type) $([math]::Round($cpu, 1)) s" }`,
    // The busiest processes, and the disk, over one second.
    String.raw`$busy = (Get-Counter '\Process(*)\% Processor Time', '\PhysicalDisk(_Total)\% Disk Time' -SampleInterval 1 -MaxSamples 1).CounterSamples | Sort-Object CookedValue -Descending | Select-Object -First 8 | ForEach-Object { "$($_.Path.Split('\')[-2]) $([math]::Round($_.CookedValue))%" }`,
    String.raw`"$state; processor time used: $($processes -join ', '); busiest now: $($busy -join ', ')"`,
  ].join("; ");
  try {
    const { stdout } = await promisify(execFile)("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
      encoding: "utf8",
      timeout: 30_000,
    });
    return stdout.trim();
  } catch (error) {
    return `(not known: ${String(error).split(/\r?\n/)[0]})`;
  }
}

async function mainPage(browser: Browser): Promise<Page> {
  // A browser reached over CDP comes with its default context.
  const context = browser.contexts()[0];
  if (!context) throw new Error("the WebView has no browser context");
  const page = context.pages()[0] ?? (await context.waitForEvent("page"));
  await page.waitForLoadState("domcontentloaded");
  return page;
}

/** The whole screen, to show dialogs the app may be waiting on when its WebView never came up. */
function desktopScreenshot(file: string) {
  const script = [
    "Add-Type -AssemblyName System.Windows.Forms, System.Drawing",
    "$area = [System.Windows.Forms.SystemInformation]::VirtualScreen",
    "$bitmap = New-Object System.Drawing.Bitmap $area.Width, $area.Height",
    "[System.Drawing.Graphics]::FromImage($bitmap).CopyFromScreen($area.Location, [System.Drawing.Point]::Empty, $area.Size)",
    `$bitmap.Save('${file.replaceAll("'", "''")}')`,
  ].join("; ");
  execFileSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], { stdio: "ignore", timeout: 30_000 });
}

/** The app's process tree with command lines (is its WebView2 running, with which arguments?). */
function processTree(pid: number): string {
  const script = [
    "$all = Get-CimInstance Win32_Process",
    `$tree = @(${pid})`,
    "do { $more = $all | Where-Object { $tree -contains $_.ParentProcessId -and $tree -notcontains $_.ProcessId }; $tree += $more.ProcessId } while ($more)",
    "$all | Where-Object { $tree -contains $_.ProcessId } | ForEach-Object { \"$($_.ProcessId) $($_.Name) $($_.CommandLine)\" }",
  ].join("; ");
  try {
    return execFileSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
      encoding: "utf8",
      timeout: 30_000,
    });
  } catch (error) {
    return `(process list failed: ${String(error)})`;
  }
}

/** The WebView2 browser process under the app with `pid`, as "<id> <name> <command line>". */
function browserCommandLine(pid: number): string {
  return processTree(pid)
    .split(/\r?\n/)
    .filter((line) => /msedgewebview2\.exe/i.test(line) && !line.includes("--type="))
    .join("\n");
}

/** The command line of the app's WebView2 browser process: the arguments WebView2 runs with. */
export function webViewCommandLine(page: Page): string {
  return browserCommandLine(appProcessId(page));
}

function stop(running: Running) {
  try {
    // The whole tree: the app, its worker and its WebView2 processes.
    execFileSync("taskkill", ["/PID", String(running.child.pid), "/T", "/F"], { stdio: "ignore" });
  } catch {
    // Already gone.
  }
  try {
    setDebuggingPolicy(null);
  } catch {
    // Not set (or already removed).
  }
  // WebView2 can hold its profile for a moment after exiting.
  rmSync(running.profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  rmSync(running.data, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
}

/**
 * Ends the app behind `page` now, with its worker and WebView2 processes, as a crash or a power
 * cut would: nothing is saved on the way out. A later `launch` can then start the app again with
 * the same data folder (`dataDir`). Its folders are still deleted when the test ends.
 */
export async function quit(page: Page): Promise<void> {
  const running = apps.get(page);
  if (!running) throw new Error("not a page from launch()");
  await running.browser?.close().catch(() => {});
  const exited = new Promise((resolve) => running.child.once("exit", resolve));
  try {
    execFileSync("taskkill", ["/PID", String(running.child.pid), "/T", "/F"], { stdio: "ignore" });
  } catch {
    // Already gone.
  }
  // The single-instance lock goes with the process: wait for it, or the next launch hands its
  // file to this one and exits.
  if (running.child.exitCode === null) await exited;
}

/**
 * Asks the app window behind `page` to close, as its close button does (WM_CLOSE): the page
 * cannot, as it has no permission to close the window. Returns once the message is posted.
 */
export async function closeAppWindow(page: Page): Promise<void> {
  const script = [
    `Add-Type -Namespace E2e -Name Window -MemberDefinition '[DllImport("user32.dll")] public static extern bool PostMessage(System.IntPtr window, uint message, System.IntPtr wParam, System.IntPtr lParam);'`,
    `$window = (Get-Process -Id ${appProcessId(page)}).MainWindowHandle`,
    "if ($window -eq [System.IntPtr]::Zero) { throw 'the app has no window' }",
    // WM_CLOSE
    "[void][E2e.Window]::PostMessage($window, 0x0010, [System.IntPtr]::Zero, [System.IntPtr]::Zero)",
  ].join("; ");
  await promisify(execFile)("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], { timeout: 30_000 });
}

/** Whether the app process behind `page` is still running. */
export function appRunning(page: Page): boolean {
  return apps.get(page)?.child.exitCode === null;
}

/**
 * Answers the file dialog the app behind `page` is showing (open, save or choose a folder, #86,
 * B2-04): types `path` into its file name box and confirms, or cancels. The dialogs are the
 * system's own, so the page cannot reach them: file-dialog.ps1 does, through UI Automation.
 * Returns once the dialog has closed.
 */
export async function answerFileDialog(page: Page, answer: { path: string } | "cancel"): Promise<string> {
  const script = path.join(import.meta.dirname, "file-dialog.ps1");
  const args = answer === "cancel" ? ["-Cancel"] : ["-Path", answer.path];
  const { stdout } = await promisify(execFile)(
    "powershell",
    ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script, "-ProcessId", String(appProcessId(page)), ...args],
    { timeout: 60_000 },
  );
  return stdout;
}

/**
 * Starts the app a second time with `file` (as Explorer does for a PDF, MVP-14). That launch hands
 * the file to the running window, which opens it in a new tab, and exits; this waits until it has.
 */
export function launchAgain(file: string): void {
  execFileSync(APP, [file], { stdio: "ignore", timeout: STARTUP_TIMEOUT_MS });
}

export type LaunchOptions = {
  /**
   * Display scaling as Windows applies it (`--force-device-scale-factor`), whatever this
   * computer's: scroll bars then take fractions of CSS pixels, as on real screens (#83).
   * CDP's emulation cannot do that: it keeps them at whole CSS pixels.
   */
  deviceScaleFactor?: number;
  /** The data folder of an earlier launch in the same test (`dataDir(page)`), after `quit`. */
  dataDir?: string;
  /** Where WebView2 writes its network log (`--log-net-log`): every request it makes (#121). */
  netLog?: string;
  /** More WebView2 arguments, for this launch only. */
  browserArguments?: string[];
};

export const test = base.extend<{
  /** Starts the app, optionally with a file to open (as a command-line argument), and returns its window's page. */
  launch: (file?: string, options?: LaunchOptions) => Promise<Page>;
}>({
  // eslint-disable-next-line no-empty-pattern -- Playwright fixtures take their dependencies as the first argument.
  launch: async ({}, provide, testInfo) => {
    const started: Running[] = [];
    await provide(async (file, options = {}) => {
      const port = await freePort();
      const profile = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-"));
      const data = options.dataDir ?? mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-data-"));
      const browserArguments = [
        `--remote-debugging-port=${port}`,
        ...(options.deviceScaleFactor ? [`--force-device-scale-factor=${options.deviceScaleFactor}`] : []),
        // Quoted only when it must be: on CI the arguments also go through `reg add`.
        ...(options.netLog
          ? [`--log-net-log=${/\s/.test(options.netLog) ? `"${options.netLog}"` : options.netLog}`]
          : []),
        ...(options.browserArguments ?? []),
      ]
        .filter(Boolean)
        .join(" ");
      setDebuggingPolicy(browserArguments);
      const child = spawn(APP, file ? [file] : [], {
        env: {
          ...process.env,
          WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: browserArguments,
          WEBVIEW2_USER_DATA_FOLDER: profile,
          PDF_READER_DATA_DIR: data,
        },
        stdio: ["ignore", "pipe", "pipe"],
      });
      const running: Running = {
        child,
        log: [],
        profile,
        data,
        launched: Date.now(),
        pendingIpc: new Map(),
        answeredIpc: new Map(),
      };
      started.push(running);
      child.stdout?.on("data", (chunk) => running.log.push(String(chunk)));
      child.stderr?.on("data", (chunk) => running.log.push(String(chunk)));
      running.browser = await connect(port, running);
      running.connected = Date.now();
      const page = await mainPage(running.browser);
      running.page = page;
      followIpc(page, running);
      // How the page's main thread spends its time, and when it stalls, from now on (#133).
      await page.addInitScript(heartbeat);
      await page.evaluate(heartbeat).catch(() => {});
      running.performance = await page.context().newCDPSession(page);
      await running.performance.send("Performance.enable").catch(() => {});
      // WebView2's start is not the test's: the test's time limit grows by what it takes.
      const budget = testInfo.timeout;
      if (budget > 0) testInfo.setTimeout(budget + PAGE_TIMEOUT_MS);
      await waitForApp(page, running);
      if (budget > 0) testInfo.setTimeout(budget + ((running.ready ?? Date.now()) - running.launched));
      dataDirs.set(page, data);
      apps.set(page, running);
      if (child.pid !== undefined) processes.set(page, child.pid);
      page.on("console", (message) => running.log.push(`[console.${message.type()}] ${message.text()}\n`));
      page.on("pageerror", (error) => running.log.push(`[pageerror] ${error.message}\n`));
      return page;
    });

    const failed = testInfo.status !== testInfo.expectedStatus;
    for (const [index, running] of started.entries()) {
      if (failed) {
        // Files in the test's output folder (uploaded by CI), also shown in the HTML report.
        const page = running.page ?? running.browser?.contexts()[0]?.pages()[0];
        const screenshot = testInfo.outputPath(`screenshot-${index}.png`);
        if (await page?.screenshot({ path: screenshot }).then(() => true, () => false)) {
          await testInfo.attach(`screenshot-${index}`, { path: screenshot, contentType: "image/png" });
        } else if (process.env.CI) {
          // No page to show: the WebView did not start. Show the whole screen instead, only on
          // CI runners: on a developer's computer it would capture their own screen.
          try {
            desktopScreenshot(screenshot);
            await testInfo.attach(`desktop-${index}`, { path: screenshot, contentType: "image/png" });
          } catch {
            // No desktop to capture.
          }
        }
        console.log(await summary(running));
        if (running.child.pid !== undefined && running.child.exitCode === null) {
          running.log.push(`\n--- process tree ---\n${processTree(running.child.pid)}`);
        }
        const log = testInfo.outputPath(`app-${index}.log`);
        writeFileSync(log, running.log.join(""));
        await testInfo.attach(`app-log-${index}`, { path: log, contentType: "text/plain" });
      }
      // Disconnects only; the app is stopped below.
      await running.browser?.close().catch(() => {});
      stop(running);
    }
  },
});

export { expect } from "@playwright/test";
