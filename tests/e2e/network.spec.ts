// The app connects to nothing on its own (AGENTS.md principle 1, ADR 0009, #121). Left to itself,
// WebView2 would: as it starts it fetches Microsoft's configuration service and looks for a proxy
// (WPAD), and a minute later its component updater asks Microsoft for updates. The app's WebView2
// arguments (src-tauri/tauri.conf.json) stop that, and WebView2's own network log, which records
// every request it makes, shows it.
import { randomUUID } from "node:crypto";
import { readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, ROOT, test, webViewCommandLine } from "./app";

/** The app's own WebView2 arguments, which the build passes to WebView2. */
const APP_ARGUMENTS: string[] = JSON.parse(readFileSync(path.join(ROOT, "src-tauri", "tauri.conf.json"), "utf8"))
  .app.windows[0].additionalBrowserArgs.split(/\s+/)
  .filter(Boolean);

/**
 * Only makes WebView2's component updater ask sooner: about 10 s after start-up instead of 60 s.
 * The app's --disable-component-update still turns it off.
 */
const COMPONENT_UPDATER_SOONER = "--component-updater=fast-update";

/** How long the app runs, from launch: the component updater would have asked well within it. */
const WATCH_MS = 25_000;

/** The app itself, and this computer: `tauri.localhost`, `ipc.localhost`, loopback. */
const LOCAL = /^((https?|wss?):\/\/)?((ipc|tauri)\.localhost|localhost|127\.0\.0\.1|\[::1\])(:\d+)?(\/|$)/;

/**
 * Every web address and host in a network log that is not local. Without their queries: those to
 * Microsoft's configuration service identify this computer's WebView2.
 */
function external(log: string): string[] {
  const urls = [...log.matchAll(/"url":"((?:https?|wss?):\/\/[^"?]+)/g)].map((match) => match[1]!);
  const hosts = [...log.matchAll(/"host":"([^"]+)"/g)].map((match) => match[1]!);
  return [...new Set([...urls, ...hosts])].filter((address) => !LOCAL.test(address));
}

test("the WebView connects to nothing while the app starts, shows a document and stays open (#121)", async ({
  launch,
}) => {
  test.setTimeout(90_000);
  const netLog = path.join(tmpdir(), `pdf-reader-netlog-${randomUUID().slice(0, 8)}.json`);
  try {
    const launched = Date.now();
    const page = await launch(corpus("benign/multi-page-10.pdf"), {
      netLog,
      browserArguments: [COMPONENT_UPDATER_SOONER],
    });
    // WebView2 runs with the app's arguments, as the build passes them.
    const commandLine = webViewCommandLine(page).split(/\s+/);
    for (const argument of APP_ARGUMENTS) {
      expect.soft(commandLine, `WebView2 runs with ${argument}`).toContain(argument);
    }
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );
    await page.waitForTimeout(Math.max(0, launched + WATCH_MS - Date.now()));
    await quit(page);
    const log = readFileSync(netLog, "utf8");
    // The log has the app's own requests, so it did record them.
    expect(log).toContain("http://tauri.localhost/");
    expect(external(log)).toEqual([]);
  } finally {
    // WebView2 can hold its log for a moment after exiting.
    rmSync(netLog, { force: true, maxRetries: 10, retryDelay: 200 });
  }
});
