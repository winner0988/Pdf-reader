// Starts the real app and drives its WebView over the Chrome DevTools Protocol (QA-02,
// docs/architecture/e2e.md). WebView2 reads its launch settings from environment variables:
// each test gets a remote debugging port on 127.0.0.1 and a fresh, temporary WebView2 profile,
// so it has its own browser process and nothing carries over between tests.

import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { get } from "node:http";
import { createServer, type AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import path from "node:path";

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
 * also set as that policy, for this executable, while the app starts.
 */
const MACHINE_POLICY = "HKLM\\Software\\Policies\\Microsoft\\Edge\\WebView2\\AdditionalBrowserArguments";

function setDebuggingPolicy(port: number | null) {
  if (!process.env.CI) return;
  const name = path.basename(APP);
  const args =
    port === null
      ? ["delete", MACHINE_POLICY, "/v", name, "/f"]
      : ["add", MACHINE_POLICY, "/v", name, "/t", "REG_SZ", "/d", `--remote-debugging-port=${port}`, "/f"];
  execFileSync("reg", args, { stdio: "ignore" });
}

async function freePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

type Running = { child: ChildProcess; log: string[]; profile: string; browser?: Browser };

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
  throw new Error(`the app's WebView did not start within ${STARTUP_TIMEOUT_MS} ms`);
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
}

export const test = base.extend<{
  /** Starts the app, optionally with a file to open (as a command-line argument), and returns its window's page. */
  launch: (file?: string) => Promise<Page>;
}>({
  // eslint-disable-next-line no-empty-pattern -- Playwright fixtures take their dependencies as the first argument.
  launch: async ({}, provide, testInfo) => {
    const started: Running[] = [];
    await provide(async (file) => {
      const port = await freePort();
      const profile = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-"));
      setDebuggingPolicy(port);
      const child = spawn(APP, file ? [file] : [], {
        env: {
          ...process.env,
          WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
          WEBVIEW2_USER_DATA_FOLDER: profile,
        },
        stdio: ["ignore", "pipe", "pipe"],
      });
      const running: Running = { child, log: [], profile };
      started.push(running);
      child.stdout?.on("data", (chunk) => running.log.push(String(chunk)));
      child.stderr?.on("data", (chunk) => running.log.push(String(chunk)));
      running.browser = await connect(port, running);
      const page = await mainPage(running.browser);
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
