/* eslint-disable no-restricted-globals, no-restricted-properties -- Testing that CSP blocks these APIs (#613) */
// Confirms in the real app that Content Security Policy (CSP) blocks every kind of outbound
// request from the frontend (AGENTS.md principles 1, 6; ADR 0009; docs/security/attack-surface.md
// section 5; #613).
import { randomUUID } from "node:crypto";
import { readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, quit, test } from "./app";
import { external } from "./netlog";

const BASE = "https://csp-check.example/";

test("the page's CSP blocks every kind of request to another site (#613)", async ({ launch }) => {
  test.setTimeout(120_000);
  const netLog = path.join(tmpdir(), `pdf-reader-netlog-${randomUUID().slice(0, 8)}.json`);
  try {
    const page = await launch(corpus("benign/single-page.pdf"), { netLog });
    await expect(page.getByRole("img", { name: strings.canvas.page(1) }).first()).toHaveAttribute(
      "data-state",
      "ready",
    );

    const { violations, outcomes } = await page.evaluate(async (base) => {
      const violations: { effectiveDirective: string; blockedURI: string }[] = [];
      const onViolation = (e: SecurityPolicyViolationEvent) => {
        violations.push({
          effectiveDirective: e.effectiveDirective,
          blockedURI: e.blockedURI,
        });
      };
      document.addEventListener("securitypolicyviolation", onViolation);

      const timeout = (ms: number, fallback = "timeout") =>
        new Promise<string>((resolve) => setTimeout(() => resolve(fallback), ms));

      const outcomes: Record<string, string> = {};
      const addedElements: Element[] = [];

      // 1. fetch
      try {
        outcomes.fetch = await Promise.race([
          fetch(base + "fetch")
            .then(() => "ok")
            .catch((e) => `failed: ${e}`),
          timeout(5000),
        ]);
      } catch (e) {
        outcomes.fetch = `failed: ${e}`;
      }

      // 2. XMLHttpRequest
      outcomes.xhr = await Promise.race([
        new Promise<string>((resolve) => {
          try {
            const xhr = new XMLHttpRequest();
            xhr.open("GET", base + "xhr");
            xhr.onload = () => resolve("loaded");
            xhr.onerror = () => resolve("error");
            xhr.onabort = () => resolve("aborted");
            xhr.send();
          } catch (e) {
            resolve(`failed: ${e}`);
          }
        }),
        timeout(5000),
      ]);

      // 3. WebSocket
      outcomes.websocket = await Promise.race([
        new Promise<string>((resolve) => {
          try {
            const ws = new WebSocket("wss://csp-check.example/ws");
            ws.onopen = () => {
              ws.close();
              resolve("open");
            };
            ws.onerror = () => resolve("error");
          } catch (e) {
            resolve(`failed: ${e}`);
          }
        }),
        timeout(5000),
      ]);

      // 4. EventSource
      outcomes.eventsource = await Promise.race([
        new Promise<string>((resolve) => {
          try {
            const es = new EventSource(base + "events");
            es.onopen = () => {
              es.close();
              resolve("open");
            };
            es.onerror = () => {
              es.close();
              resolve("error");
            };
          } catch (e) {
            resolve(`failed: ${e}`);
          }
        }),
        timeout(5000),
      ]);

      // 5. sendBeacon
      try {
        const sent = navigator.sendBeacon(base + "beacon", "x");
        outcomes.sendBeacon = sent ? "sent" : "blocked";
      } catch (e) {
        outcomes.sendBeacon = `failed: ${e}`;
      }

      // 6. 圖片 (Image)
      const img = new Image();
      addedElements.push(img);
      outcomes.image = await Promise.race([
        new Promise<string>((resolve) => {
          img.onload = () => resolve("loaded");
          img.onerror = () => resolve("error");
          try {
            img.src = base + "img.png";
            document.body.appendChild(img);
          } catch (e) {
            resolve(`failed: ${e}`);
          }
        }),
        timeout(5000),
      ]);

      // 7. 指令碼 (script)
      const script = document.createElement("script");
      addedElements.push(script);
      outcomes.script = await Promise.race([
        new Promise<string>((resolve) => {
          script.onload = () => resolve("loaded");
          script.onerror = () => resolve("error");
          try {
            script.src = base + "script.js";
            document.head.appendChild(script);
          } catch (e) {
            resolve(`failed: ${e}`);
          }
        }),
        timeout(5000),
      ]);

      // 8. 樣式表 (stylesheet)
      const link = document.createElement("link");
      link.rel = "stylesheet";
      addedElements.push(link);
      outcomes.stylesheet = await Promise.race([
        new Promise<string>((resolve) => {
          link.onload = () => resolve("loaded");
          link.onerror = () => resolve("error");
          try {
            link.href = base + "style.css";
            document.head.appendChild(link);
          } catch (e) {
            resolve(`failed: ${e}`);
          }
        }),
        timeout(5000),
      ]);

      // 9. 字型 (FontFace)
      try {
        const font = new FontFace("csp-check", "url(" + base + "font.woff2)");
        outcomes.font = await Promise.race([
          font
            .load()
            .then(() => "loaded")
            .catch((e) => `failed: ${e}`),
          timeout(5000),
        ]);
      } catch (e) {
        outcomes.font = `failed: ${e}`;
      }

      // 10. CSS 的 url()
      const div = document.createElement("div");
      div.style.backgroundImage = "url(" + base + "background.png)";
      addedElements.push(div);
      document.body.appendChild(div);
      void getComputedStyle(div).backgroundImage;
      outcomes.cssUrl = "done";

      // 11. iframe
      const iframe = document.createElement("iframe");
      addedElements.push(iframe);
      outcomes.iframe = await Promise.race([
        new Promise<string>((resolve) => {
          iframe.onload = () => resolve("loaded");
          iframe.onerror = () => resolve("error");
          try {
            iframe.src = base + "frame";
            document.body.appendChild(iframe);
          } catch (e) {
            resolve(`failed: ${e}`);
          }
        }),
        timeout(5000),
      ]);

      // 等 1 秒讓非同步的違規事件都到
      await new Promise((resolve) => setTimeout(resolve, 1000));

      for (const el of addedElements) {
        el.remove();
      }
      document.removeEventListener("securitypolicyviolation", onViolation);

      return { violations, outcomes };
    }, BASE);

    const checks: { kind: string; directive: string; url: string }[] = [
      { kind: "fetch", directive: "connect-src", url: `${BASE}fetch` },
      { kind: "XMLHttpRequest", directive: "connect-src", url: `${BASE}xhr` },
      { kind: "WebSocket", directive: "connect-src", url: "wss://csp-check.example/ws" },
      { kind: "EventSource", directive: "connect-src", url: `${BASE}events` },
      { kind: "sendBeacon", directive: "connect-src", url: `${BASE}beacon` },
      { kind: "圖片", directive: "img-src", url: `${BASE}img.png` },
      { kind: "指令碼", directive: "script-src-elem", url: `${BASE}script.js` },
      { kind: "樣式表", directive: "style-src-elem", url: `${BASE}style.css` },
      { kind: "字型", directive: "font-src", url: `${BASE}font.woff2` },
      { kind: "CSS 的 url()", directive: "img-src", url: `${BASE}background.png` },
      { kind: "iframe", directive: "frame-src", url: `${BASE}frame` },
    ];

    for (const check of checks) {
      const matched = violations.some(
        (v) => v.effectiveDirective === check.directive && v.blockedURI.startsWith(check.url),
      );
      expect.soft(matched, `${check.kind} blocked by ${check.directive}`).toBe(true);
    }

    expect.soft(outcomes.fetch, "fetch outcome").toMatch(/^failed/);

    await quit(page);
    const log = readFileSync(netLog, "utf8");
    expect(log).toContain("http://tauri.localhost/");
    expect(external(log)).toEqual([]);
  } finally {
    rmSync(netLog, { force: true, maxRetries: 10, retryDelay: 200 });
  }
});
