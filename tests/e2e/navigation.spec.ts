// The main window's WebView only navigates to the app's own pages (Issue #424, AGENTS.md
// principle 1 and principle 6). Top-level navigation to external sites, fake domains, and
// arbitrary schemes is blocked by `on_navigation`; downloads are blocked by `on_download`.
import { randomUUID } from "node:crypto";
import { existsSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, test } from "./app";

/** The app itself, and this computer: `tauri.localhost`, `ipc.localhost`, loopback. */
const LOCAL = /^((https?|wss?):\/\/)?((ipc|tauri)\.localhost|localhost|127\.0\.0\.1|\[::1\])(:\d+)?(\/|$)/;

type NetLogData = {
  constants?: {
    logEventTypes?: Record<string, number>;
    netErrors?: Record<string, number>;
  };
  events?: Array<{
    source?: { id: number; type: number };
    type?: number;
    params?: Record<string, unknown>;
  }>;
};

/**
 * Chromium's `--log-net-log` continuously streams JSON events to disk. When `quit(page)` forcibly
 * terminates the app process tree, the file may end mid-stream without the closing `]}`.
 * `parseNetLog` trims to the last complete event object and closes the JSON.
 */
function parseNetLog(log: string): NetLogData {
  try {
    return JSON.parse(log) as NetLogData;
  } catch {
    const lastBrace = log.lastIndexOf("}");
    if (lastBrace > 0) {
      try {
        return JSON.parse(log.slice(0, lastBrace + 1) + "]}") as NetLogData;
      } catch {
        // Incomplete / unparseable
      }
    }
    return { constants: {}, events: [] };
  }
}

/**
 * Any external web address in the network log that was actually transmitted (sent HTTP request
 * headers or connected to a remote server).
 *
 * In Chromium, setting `location.href` in JavaScript causes the network stack to create an initial
 * `URL_REQUEST` entry in NetLog before WebView2's `NavigationStarting` event fires.
 * When `on_navigation` blocks navigation (returning `false`), WebView2 immediately cancels the
 * request (`ERR_ABORTED`, net_error = -3) before any HTTP transaction (`HTTP_TRANSACTION_SEND_REQUEST`)
 * is created or any socket is connected.
 */
function externalTransmitted(log: string): string[] {
  const data = parseNetLog(log);

  const sendHeaderType = data.constants?.logEventTypes?.HTTP_TRANSACTION_SEND_REQUEST_HEADERS;
  const sendRequestType = data.constants?.logEventTypes?.HTTP_TRANSACTION_SEND_REQUEST;
  const socketConnectType = data.constants?.logEventTypes?.TCP_CONNECT;

  const requestUrls = new Map<number, string>();
  for (const event of data.events ?? []) {
    if (event.params?.url && typeof event.params.url === "string" && event.source?.id !== undefined) {
      requestUrls.set(event.source.id, event.params.url);
    }
  }

  const transmitted: string[] = [];
  for (const event of data.events ?? []) {
    if (
      event.type !== undefined &&
      (event.type === sendHeaderType || event.type === sendRequestType || event.type === socketConnectType)
    ) {
      const url = event.source?.id !== undefined ? requestUrls.get(event.source.id) : undefined;
      if (url && !LOCAL.test(url)) {
        transmitted.push(url);
      }
    }
  }
  return [...new Set(transmitted)];
}

test("the WebView refuses navigation to external URLs and fake domains, and net log stays empty (#424)", async ({
  launch,
}) => {
  test.setTimeout(60_000);
  const netLog = path.join(tmpdir(), `pdf-reader-nav-netlog-${randomUUID().slice(0, 8)}.json`);
  try {
    const page = await launch(corpus("benign/multi-page-10.pdf"), { netLog });
    const firstPage = page.getByRole("img", { name: strings.canvas.page(1) }).first();
    await expect(firstPage).toHaveAttribute("data-state", "ready");

    // 1. Attempt navigation to an external HTTPS URL
    await page.evaluate(() => {
      location.href = "https://example.invalid/";
    });
    await page.waitForTimeout(500);
    expect(await page.evaluate(() => location.href)).toContain("http://tauri.localhost");
    await expect(firstPage).toHaveAttribute("data-state", "ready");

    // 2. Attempt navigation to a domain starting with tauri.localhost
    await page.evaluate(() => {
      location.href = "http://tauri.localhost.evil.example/";
    });
    await page.waitForTimeout(500);
    expect(await page.evaluate(() => location.href)).toContain("http://tauri.localhost");
    await expect(firstPage).toHaveAttribute("data-state", "ready");

    // 3. Attempt navigation to a file: URL
    await page.evaluate(() => {
      location.href = "file:///C:/Windows/win.ini";
    });
    await page.waitForTimeout(500);
    expect(await page.evaluate(() => location.href)).toContain("http://tauri.localhost");
    await expect(firstPage).toHaveAttribute("data-state", "ready");

    await quit(page);

    const log = readFileSync(netLog, "utf8");
    expect(log).toContain("http://tauri.localhost/");
    // No external requests were transmitted (no headers sent, no remote socket connected)
    expect(externalTransmitted(log)).toEqual([]);

    // Verify raw log contains no HTTP request headers to external hosts
    expect(log).not.toContain("Host: example.invalid");
    expect(log).not.toContain("Host: tauri.localhost.evil.example");
  } finally {
    rmSync(netLog, { force: true, maxRetries: 10, retryDelay: 200 });
  }
});

test("the WebView refuses downloads triggered by data: or blob: links (#424)", async ({ launch }) => {
  test.setTimeout(60_000);
  const downloadsFolder = path.join(process.env.USERPROFILE ?? "", "Downloads");
  const dataFile = path.join(downloadsFolder, `leak-data-${randomUUID().slice(0, 8)}.txt`);
  const blobFile = path.join(downloadsFolder, `leak-blob-${randomUUID().slice(0, 8)}.txt`);

  try {
    const page = await launch(corpus("benign/multi-page-10.pdf"));
    const firstPage = page.getByRole("img", { name: strings.canvas.page(1) }).first();
    await expect(firstPage).toHaveAttribute("data-state", "ready");

    // Attempt data: URI download
    const dataFileName = path.basename(dataFile);
    await page.evaluate((fileName) => {
      const a = document.createElement("a");
      a.href = "data:text/plain;charset=utf-8,malicious_payload";
      a.download = fileName;
      document.body.appendChild(a);
      a.click();
      a.remove();
    }, dataFileName);

    // Attempt blob: URI download
    const blobFileName = path.basename(blobFile);
    await page.evaluate((fileName) => {
      const blob = new Blob(["blob payload"], { type: "text/plain" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = fileName;
      document.body.appendChild(a);
      a.click();
      a.remove();
    }, blobFileName);

    await page.waitForTimeout(1_000);

    // Verify neither file was written to the Downloads folder
    expect(existsSync(dataFile)).toBe(false);
    expect(existsSync(blobFile)).toBe(false);

    // The app remains fully functional
    await expect(firstPage).toHaveAttribute("data-state", "ready");
    await quit(page);
  } finally {
    rmSync(dataFile, { force: true });
    rmSync(blobFile, { force: true });
  }
});
