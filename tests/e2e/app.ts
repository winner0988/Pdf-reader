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

function stop(running: Running) {
  try {
    // The whole tree: the app, its worker and its WebView2 processes.
    execFileSync("taskkill", ["/PID", String(running.child.pid), "/T", "/F"], { stdio: "ignore" });
  } catch {
    // Already gone.
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
