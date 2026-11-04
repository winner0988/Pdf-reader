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

/**
 * Every web address and host in a network log that is not local. Without their queries.
 */
function external(log: string): string[] {
  const urls = [...log.matchAll(/"url":"((?:https?|wss?):\/\/[^"?]+)/g)].map((match) => match[1]!);
  const hosts = [...log.matchAll(/"host":"([^"]+)"/g)].map((match) => match[1]!);
  return [...new Set([...urls, ...hosts])].filter((address) => !LOCAL.test(address));
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
    expect(external(log)).toEqual([]);
    expect(log).not.toContain("example.invalid");
    expect(log).not.toContain("evil.example");
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
