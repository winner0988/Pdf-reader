import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const patternsFile = path.resolve(__dirname, "forbidden-patterns.txt");

function loadRules() {
  const content = fs.readFileSync(patternsFile, "utf-8");
  const lines = content.split(/\r?\n/);
  const rules = [];

  for (const rawLine of lines) {
    if (!rawLine.trim() || rawLine.trim().startsWith("#")) {
      continue;
    }
    const ruleText = rawLine.includes(" @only ")
      ? rawLine.split(" @only ")[0]
      : rawLine;

    const converted = ruleText.replace(/\[\[:space:\]\]/g, "\\s");
    if (/\[\[:/.test(converted)) {
      assert.fail(`POSIX character class not converted in rule: ${ruleText}`);
    }

    rules.push({ raw: ruleText, regex: new RegExp(converted, "i") });
  }
  return rules;
}

test("every rule compiles", () => {
  const rules = loadRules();
  assert.ok(rules.length > 0, "no rules loaded from forbidden-patterns.txt");
  for (const { raw, regex } of rules) {
    assert.ok(regex instanceof RegExp, `rule '${raw}' is not a valid RegExp`);
  }
});

test("each telemetry package is caught (#614)", () => {
  const rules = loadRules();
  const samples = [
    '"@elastic/apm-rum": "5.17.0"',
    '"@grafana/faro-web-sdk": "1.0.0"',
    '"@honeybadger-io/js": "6.0.0"',
    '"raygun4js": "3.0.0"',
    '"trackjs": "3.0.0"',
    '"@appsignal/javascript": "1.0.0"',
    '"@splunk/otel-web": "0.20.0"',
    '"aws-rum-web": "1.0.0"',
    '"highlight.run": "9.0.0"',
    '"@highlight-run/react": "1.0.0"',
    '"@openreplay/tracker": "16.0.0"',
    '"@aptabase/web": "0.4.0"',
    'tauri-plugin-aptabase = "1"',
    '"@fullstory/browser": "2.0.0"',
    '"@hotjar/browser": "1.0.0"',
    '"@microsoft/clarity": "1.0.0"',
    '<script src="https://www.clarity.ms/tag/x">',
    '"smartlook-client": "10.0.0"',
    '<script src="https://cdn.mouseflow.com/projects/x.js">',
    '<script src="https://cdn.heapanalytics.com/js/heap-1.js">',
    '"plausible-tracker": "0.3.0"',
    '<script src="https://plausible.io/js/script.js">',
    '"@datapunt/matomo-tracker-js": "0.5.0"',
    '"fathom-client": "3.0.0"',
    '<script src="https://cdn.usefathom.com/script.js">',
    '<script src="https://cdn.pendo.io/agent/static/x/pendo.js">',
    '"@sentry/browser": "8.0.0"',
    'tauri-plugin-http = "2"',
  ];

  for (const sample of samples) {
    const matched = rules.some(({ regex }) => regex.test(sample));
    assert.ok(matched, `Expected sample to be caught, but was not: ${sample}`);
  }
});

test("ordinary words and identifiers are not caught", () => {
  const rules = loadRules();
  const allowedSamples = [
    ".scrollbar { width: 8px; }",
    "a plausible reason",
    "for clarity",
    "heap memory",
    "fathom the layout",
    "appendOnly(list)",
    "const mouseFlow = 0;",
    "highlight run of text",
    "elasticity",
    "track.js",
    "const sentry = guard;",
  ];

  for (const sample of allowedSamples) {
    const matchingRule = rules.find(({ regex }) => regex.test(sample));
    assert.equal(
      matchingRule,
      undefined,
      `Expected sample not to match any rule, but '${sample}' matched '${matchingRule?.raw}'`
    );
  }
});
