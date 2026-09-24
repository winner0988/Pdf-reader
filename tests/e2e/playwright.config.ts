// End-to-end tests against the real Windows app (QA-02, docs/architecture/e2e.md).
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  // One app at a time: each test starts its own app, worker and WebView2 browser process.
  workers: 1,
  fullyParallel: false,
  // A flaky end-to-end test is a bug to fix, not to retry away.
  retries: 0,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  forbidOnly: !!process.env.CI,
  reporter: [["list"], ["html", { open: "never", outputFolder: "playwright-report" }]],
  // Screenshots and app logs of failed tests land here (uploaded by CI).
  outputDir: "test-results",
});
