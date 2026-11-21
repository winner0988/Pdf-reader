/** The app itself, and this computer: `tauri.localhost`, `ipc.localhost`, loopback. */
export const LOCAL = /^((https?|wss?):\/\/)?((ipc|tauri)\.localhost|localhost|127\.0\.0\.1|\[::1\])(:\d+)?(\/|$)/;

/**
 * Every web address and host in a network log that is not local. Without their queries: those to
 * Microsoft's configuration service identify this computer's WebView2.
 */
export function external(log: string): string[] {
  const urls = [...log.matchAll(/"url":"((?:https?|wss?):\/\/[^"?]+)/g)].map((match) => match[1]!);
  const hosts = [...log.matchAll(/"host":"([^"]+)"/g)].map((match) => match[1]!);
  return [...new Set([...urls, ...hosts])].filter((address) => !LOCAL.test(address));
}
