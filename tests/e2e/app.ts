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

import { chromium, test as base, type Browser, type Page } from "@playwright/test";

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

type Running = { child: ChildProcess; log: string[]; profile: string; data: string; browser?: Browser };

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
      const running: Running = { child, log: [], profile, data };
      started.push(running);
      child.stdout?.on("data", (chunk) => running.log.push(String(chunk)));
      child.stderr?.on("data", (chunk) => running.log.push(String(chunk)));
      running.browser = await connect(port, running);
      const page = await mainPage(running.browser);
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
        const page = running.browser?.contexts()[0]?.pages()[0];
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
