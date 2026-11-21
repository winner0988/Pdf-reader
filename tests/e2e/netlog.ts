/** The app itself, and this computer: `tauri.localhost`, `ipc.localhost`, loopback. */
export const LOCAL = /^((https?|wss?):\/\/)?((ipc|tauri)\.localhost|localhost|127\.0\.0\.1|\[::1\])(:\d+)?(\/|$)/;

type NetLogData = {
  constants?: {
    logSourceType?: Record<string, number>;
  };
  events?: Array<{
    source?: { id: number; type: number };
    params?: Record<string, unknown>;
  }>;
};

/**
 * Chromium's `--log-net-log` continuously streams JSON events to disk. When `quit(page)` forcibly
 * terminates the app process tree, the file may end mid-stream without the closing `]}`.
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
    return {};
  }
}

/**
 * Every web address and host in a network log that is not local. Without their queries: those to
 * Microsoft's configuration service identify this computer's WebView2.
 *
 * Speculative preconnections (which Chromium initiates automatically for navigation candidates
 * before CSP frame-src evaluation) are excluded when no actual URL_REQUEST is ever dispatched.
 */
export function external(log: string): string[] {
  const data = parseNetLog(log);
  if (!data.events) {
    const urls = [...log.matchAll(/"url":"((?:https?|wss?):\/\/[^"?]+)/g)].map((match) => match[1]!);
    const hosts = [...log.matchAll(/"host":"([^"]+)"/g)].map((match) => match[1]!);
    return [...new Set([...urls, ...hosts])].filter((address) => !LOCAL.test(address));
  }

  const preconnectUrls = new Set<string>();
  const preconnectHosts = new Set<string>();
  for (const event of data.events) {
    if (event.params?.is_preconnect === true && typeof event.params.url === "string") {
      const clean = event.params.url.split("?")[0]!;
      preconnectUrls.add(clean);
      try {
        const u = new URL(clean);
        preconnectHosts.add(u.hostname);
        preconnectHosts.add(u.origin);
      } catch {
        // Not a valid URL
      }
    }
  }

  const urlRequestType = data.constants?.logSourceType?.URL_REQUEST ?? 1;
  const actualRequestUrls = new Set<string>();
  const actualRequestHosts = new Set<string>();

  for (const event of data.events) {
    if (event.source?.type === urlRequestType && event.params) {
      if (typeof event.params.url === "string") {
        actualRequestUrls.add(event.params.url.split("?")[0]!);
      }
      if (typeof event.params.host === "string") {
        actualRequestHosts.add(event.params.host);
      }
    }
  }

  const allUrls = [...log.matchAll(/"url":"((?:https?|wss?):\/\/[^"?]+)/g)].map((match) => match[1]!);
  const allHosts = [...log.matchAll(/"host":"([^"]+)"/g)].map((match) => match[1]!);
  const all = [...new Set([...allUrls, ...allHosts])].filter((address) => !LOCAL.test(address));

  return all.filter((address) => {
    if (actualRequestUrls.has(address) || actualRequestHosts.has(address)) return true;
    if (preconnectUrls.has(address) || preconnectHosts.has(address)) return false;
    return true;
  });
}
