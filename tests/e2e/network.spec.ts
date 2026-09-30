// The app connects to nothing on its own (AGENTS.md principle 1, ADR 0009, #121). Left to itself,
// WebView2 would: as it starts it fetches Microsoft's configuration service and looks for a proxy
// (WPAD). The app's WebView2 arguments (src-tauri/tauri.conf.json) stop that, and WebView2's own
// network log, which records every request it makes, shows it.
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, test } from "./app";

/** The app itself, and this computer: `tauri.localhost`, `ipc.localhost`, loopback. */
const LOCAL = /^((https?|wss?):\/\/)?((ipc|tauri)\.localhost|localhost|127\.0\.0\.1|\[::1\])(:\d+)?(\/|$)/;

/** Every web address and host in a network log that is not local. */
function external(log: string): string[] {
  const urls = [...log.matchAll(/"url":"((?:https?|wss?):\/\/[^"]+)"/g)].map((match) => match[1]!);
  const hosts = [...log.matchAll(/"host":"([^"]+)"/g)].map((match) => match[1]!);
  return [...new Set([...urls, ...hosts])].filter((address) => !LOCAL.test(address));
}

test("the WebView connects to nothing while the app starts and shows a document (#121)", async ({ launch }) => {
  const folder = mkdtempSync(path.join(tmpdir(), "pdf-reader-e2e-netlog-"));
  try {
    const netLog = path.join(folder, "netlog.json");
    const page = await launch(corpus("benign/multi-page-10.pdf"), { netLog });
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    // WebView2's own requests start within a second or two; give the log time to be written.
    await page.waitForTimeout(10_000);
    await quit(page);
    const log = readFileSync(netLog, "utf8");
    // The log has the app's own requests, so it did record them.
    expect(log).toContain("http://tauri.localhost/");
    expect(external(log)).toEqual([]);
  } finally {
    // WebView2 can hold its log for a moment after exiting.
    rmSync(folder, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  }
});
