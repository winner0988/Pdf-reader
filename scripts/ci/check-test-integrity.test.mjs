import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { checkChange, checkChanges, diffLines, parseWorkflow } from "./check-test-integrity.mjs";

const SCRIPT = fileURLToPath(new URL("./check-test-integrity.mjs", import.meta.url));
const WORKFLOWS = fileURLToPath(new URL("../../.github/workflows/", import.meta.url));

/** `text` with `from` replaced by `to`; the fixture must contain `from`. */
function edit(text, from, to) {
  assert.ok(text.includes(from), `the fixture has ${JSON.stringify(from)}`);
  return text.replace(from, to);
}

const problems = (path, base, head, oldPath = null) => checkChange({ path, oldPath, base, head });

/** Asserts that the change is reported, with a message matching `message`. */
function found(path, base, head, message) {
  const list = problems(path, base, head);
  assert.ok(list.length > 0, `expected a problem in ${path}`);
  if (message) assert.ok(list.some((p) => message.test(p.message)), `${message} in ${JSON.stringify(list)}`);
  return list;
}

function passes(path, base, head) {
  assert.deepEqual(problems(path, base, head), []);
}

// --- Line diff ---------------------------------------------------------------------------------

/** Length of a longest common subsequence, the size a minimal diff keeps. */
function lcsLength(a, b) {
  const row = new Array(b.length + 1).fill(0);
  for (let i = 1; i <= a.length; i++) {
    let diagonal = 0;
    for (let j = 1; j <= b.length; j++) {
      const up = row[j];
      row[j] = a[i - 1] === b[j - 1] ? diagonal + 1 : Math.max(row[j], row[j - 1]);
      diagonal = up;
    }
  }
  return row[b.length];
}

test("diffLines keeps a longest common subsequence and reports the rest", () => {
  let seed = 7;
  const random = (n) => {
    seed = (seed * 1103515245 + 12345) % 2147483648;
    return Math.floor((seed / 2147483648) * n);
  };
  for (let round = 0; round < 2000; round++) {
    const a = Array.from({ length: random(14) }, () => "abcd"[random(4)]);
    const b = Array.from({ length: random(14) }, () => "abcd"[random(4)]);
    const { removed, added } = diffLines(a, b);
    for (const [list, length] of [[removed, a.length], [added, b.length]]) {
      assert.deepEqual(list, [...new Set(list)].sort((p, q) => p - q));
      assert.ok(list.every((n) => n >= 1 && n <= length));
    }
    const keptA = a.filter((_, i) => !removed.includes(i + 1));
    const keptB = b.filter((_, j) => !added.includes(j + 1));
    assert.deepEqual(keptA, keptB, `${a.join("")} -> ${b.join("")}`);
    assert.equal(keptA.length, lcsLength(a, b), `${a.join("")} -> ${b.join("")}`);
  }
});

test("diffLines reports the whole middle when a file changes beyond its limit", () => {
  // A minimal diff would keep "keep"; past 4000 edits everything between the common start and end counts.
  const side = (name) => Array.from({ length: 2100 }, (_, i) => `${name} ${i}`);
  const a = ["same", ...side("old"), "keep", ...side("old 2"), "end"];
  const b = ["same", ...side("new"), "keep", ...side("new 2"), "end"];
  const { removed, added } = diffLines(a, b);
  assert.equal(removed.length, 4201);
  assert.equal(added.length, 4201);
  assert.deepEqual([removed[0], removed.at(-1)], [2, 4202]);
});

// --- The six cases of #630, and ordinary changes -------------------------------------------------

const RUST = `#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_request_leaves_the_worker() {
        assert!(network_calls().is_empty());
    }
}
`;

const FRONTEND_TEST = `import { describe, expect, it } from "vitest";

describe("viewer", () => {
  it("never calls fetch", () => {
    expect(fetchCalls()).toEqual([]);
  });
});
`;

const SPEC = `import { expect, test } from "./fixtures";

test("the app makes no network request", async ({ launch }) => {
  const app = await launch();
  await expect(app.requests()).resolves.toEqual([]);
});
`;

const PLAYWRIGHT = `export default defineConfig({
  testDir: ".",
  workers: 1,
  // A flaky end-to-end test is a bug to fix, not to retry away.
  retries: 0,
  forbidOnly: !!process.env.CI,
});
`;

const WORKFLOW = `name: CI

on:
  push:
    branches: [main]
  pull_request:
    paths:
      - "src/**"

jobs:
  detect:
    runs-on: ubuntu-latest
    outputs:
      rust: \${{ steps.detect.outputs.rust }}
    steps:
      - id: detect
        run: echo "rust=true" >> "$GITHUB_OUTPUT"

  guardrails:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          persist-credentials: false
      - name: Forbidden telemetry / network dependencies
        run: bash scripts/ci/check-forbidden.sh
      - name: Guard script tests
        run: node --test scripts/ci/*.test.mjs

  rust:
    needs: detect
    if: needs.detect.outputs.rust == 'true'
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - name: Test
        run: cargo test --workspace --locked
      - name: Upload logs
        if: failure()
        run: echo "upload"

  e2e:
    runs-on: windows-latest
    steps:
      - name: E2E tests
        run: pnpm e2e
`;

const FORBIDDEN = `# Telemetry and crash reporting SDKs
@sentry/
posthog
`;

test("#630 case 1: #[ignore] on the Rust test that would catch the change", () => {
  const head = edit(RUST, "    #[test]\n", "    #[test]\n    #[ignore]\n");
  const [problem] = found("crates/pdf_worker/src/serve.rs", RUST, head, /Rust code or tests/);
  assert.equal(problem.line, 6);
  assert.match(problem.message, /#\[ignore\]/);
});

test("#630 case 2: a frontend test turned into it.skip", () => {
  const head = edit(FRONTEND_TEST, '  it("never calls fetch"', '  it.skip("never calls fetch"');
  const [problem] = found("src/features/viewer/net.test.ts", FRONTEND_TEST, head, /\(skip\)/);
  assert.equal(problem.line, 4);
});

test("#630 case 3: the E2E network test turned into test.skip", () => {
  const head = edit(SPEC, 'test("the app makes', 'test.skip("the app makes');
  found("tests/e2e/network.spec.ts", SPEC, head, /keeps tests from running/);
});

test("#630 case 4: playwright.config.ts retries: 2", () => {
  const head = edit(PLAYWRIGHT, "retries: 0,", "retries: 2,");
  const list = found("tests/e2e/playwright.config.ts", PLAYWRIGHT, head, /retrying/);
  assert.deepEqual(list.map((p) => p.line), [5]);
});

test("#630 case 5: continue-on-error: true on the E2E job", () => {
  const head = edit(WORKFLOW, "  e2e:\n    runs-on: windows-latest\n", "  e2e:\n    runs-on: windows-latest\n    continue-on-error: true\n");
  const [problem] = found(".github/workflows/ci.yml", WORKFLOW, head, /continue-on-error: true lets job "e2e"/);
  assert.equal(problem.line, head.split("\n").indexOf("    continue-on-error: true") + 1);
});

test("#630 case 6: the @sentry/ rule removed from forbidden-patterns.txt", () => {
  const head = edit(FORBIDDEN, "@sentry/\n", "");
  found("scripts/ci/forbidden-patterns.txt", FORBIDDEN, head, /forbidden pattern is removed \(line 2 on the base\): @sentry\//);
});

test("ordinary PRs pass: a new test, a changed test, a new forbidden pattern", () => {
  passes("crates/pdf_worker/src/serve.rs", RUST, edit(RUST, "}\n}\n", "}\n\n    #[test]\n    fn renders_a_page() {\n        assert!(render(1).is_ok());\n    }\n}\n"));
  passes("src/features/viewer/net.test.ts", FRONTEND_TEST, edit(FRONTEND_TEST, "toEqual([])", "toHaveLength(0)"));
  passes("tests/e2e/network.spec.ts", SPEC, `${SPEC}\ntest("a second test", async ({ launch }) => {\n  await launch();\n});\n`);
  passes("scripts/ci/forbidden-patterns.txt", FORBIDDEN, `${FORBIDDEN}mixpanel\n`);
  passes("tests/e2e/playwright.config.ts", PLAYWRIGHT, edit(PLAYWRIGHT, "workers: 1,", "workers: 2,"));
});

// --- Rust ----------------------------------------------------------------------------------------

test("Rust: the ways to keep a test from running are found", () => {
  for (const attribute of ['#[ignore = "flaky"]', "#[cfg_attr(windows, ignore)]", "#[cfg(any())]"]) {
    found("crates/x/tests/a.rs", RUST, edit(RUST, "    #[test]\n", `    #[test]\n    ${attribute}\n`), /Rust code or tests/);
  }
});

test("Rust: #[should_panic] on an existing test is found, on a new one it passes", () => {
  found("crates/x/src/lib.rs", RUST, edit(RUST, "    #[test]\n", "    #[test]\n    #[should_panic]\n"), /should_panic/);
  const newTest = '}\n\n    #[test]\n    #[should_panic(expected = "too large")]\n    fn refuses_a_huge_page() {\n        render(u32::MAX);\n    }\n}\n';
  passes("crates/x/src/lib.rs", RUST, edit(RUST, "}\n}\n", newTest));
});

// --- Test scripts --------------------------------------------------------------------------------

test("test files: skipped, focused and expected-to-fail tests are found", () => {
  for (const [from, to] of [
    ['  it("never calls fetch"', '  it.only("never calls fetch"'],
    ['describe("viewer"', 'describe.skip("viewer"'],
    ['  it("never calls fetch"', '  it.fails("never calls fetch"'],
    ['  it("never calls fetch"', '  it.skipIf(process.env.CI)("never calls fetch"'],
    ['  it("never calls fetch"', '  xit("never calls fetch"'],
    ['  it("never calls fetch", () => {', '  it("never calls fetch", { skip: true }, () => {'],
    ["    expect(fetchCalls())", "    ctx.skip();\n    expect(fetchCalls())"],
    ['  it("never calls fetch"', '  it.todo("never calls fetch");\n  it("old"'],
  ]) {
    found("src/a.test.ts", FRONTEND_TEST, edit(FRONTEND_TEST, from, to), /keeps tests from running/);
  }
  for (const [from, to] of [
    ['test("the app', 'test.fixme("the app'],
    ["  const app = await launch();\n", "  test.fail();\n  const app = await launch();\n"],
    ["  const app = await launch();\n", "  test.skip();\n  const app = await launch();\n"],
    ["  const app = await launch();\n", '  test.skip(!!process.env.CI, "slow");\n  const app = await launch();\n'],
    ["  const app = await launch();\n", "  testInfo.skip(\n    process.platform === \"win32\",\n    \"not here\",\n  );\n  const app = await launch();\n"],
  ]) {
    found("tests/e2e/network.spec.ts", SPEC, edit(SPEC, from, to), /keeps tests from running/);
  }
});

const NEW_GATED_TEST = `
test("signs with a certificate of this computer", async ({ launch }) => {
  test.skip(!MAY_IMPORT, WHY_NOT);
  const app = await launch();
  await app.sign();
  await expect(app.signatures()).resolves.toHaveLength(1);
});

test("forgets every remembered password", async ({
  launch,
}) => {
  test.skip(
    !MAY_FORGET_ALL,
    "it forgets every remembered password: only on a runner that is thrown away",
  );
  const app = await launch();
  await app.forgetAll();
});
`;

test("test files: a conditional skip in a new test, or atop a new file, passes", () => {
  passes("tests/e2e/network.spec.ts", SPEC, SPEC + NEW_GATED_TEST);
  passes("tests/e2e/signing.spec.ts", null, `const MAY_IMPORT = !!process.env.CI;\ntest.skip(!MAY_IMPORT, "imports a certificate");\n${SPEC}`);
});

test("test files: a conditional skip in an existing test, or atop an existing file, is found", () => {
  const renamed = edit(SPEC, 'test("the app makes no network request", async ({ launch }) => {\n', 'test("the app makes no request", async ({ launch }) => {\n  test.skip(!MAY_RUN, WHY);\n');
  found("tests/e2e/network.spec.ts", SPEC, renamed, /keeps tests from running/);
  found("tests/e2e/network.spec.ts", SPEC, edit(SPEC, "\ntest(", "\ntest.skip(!MAY_RUN, WHY);\n\ntest("), /keeps tests from running/);
});

test("test files: titles, comments, props and helpers that look like skips pass", () => {
  for (const [from, to] of [
    ['  it("never calls fetch", () => {\n', '  it("pages fitted to a rounded width still fit (#83)", () => {\n'],
    ["    expect(fetchCalls())", "    // 3. Click retry: the document recovers in a new worker\n    expect(fetchCalls())"],
    ["    expect(fetchCalls())", "    // Not it.skip(): a flaky test is a bug to fix (AGENTS.md).\n    expect(fetchCalls())"],
    ["    expect(fetchCalls())", '    await fake.fail({ code: "cancelled", message: "" });\n    expect(fetchCalls())'],
    ["    expect(fetchCalls())", "    render(<Tabs retry={vi.fn()} />, { retry: vi.fn(() => Promise.resolve()) });\n    expect(fetchCalls())"],
    ["    expect(fetchCalls())", '    await checkA11y(container, { skip: { region: "a menu is a pop-up" } });\n    expect(fetchCalls())'],
    ["    expect(fetchCalls())", '    expect(log).toContain("test.skip(");\n    expect(fetchCalls())'],
  ]) {
    passes("src/a.test.tsx", FRONTEND_TEST, edit(FRONTEND_TEST, from, to));
  }
});

test("test files: retries in test code are found, other uses of the word pass", () => {
  found("tests/e2e/a.spec.ts", SPEC, `${SPEC}test.describe.configure({ retries: 2 });\n`, /retrying/);
  found("src/a.test.ts", FRONTEND_TEST, edit(FRONTEND_TEST, '"never calls fetch", () =>', '"never calls fetch", { retry: process.env.CI ? 3 : 0 }, () =>'), /retrying/);
  passes("tests/e2e/a.spec.ts", SPEC, `${SPEC}test.describe.configure({ retries: 0 });\n`);
});

// --- Test configs --------------------------------------------------------------------------------

test("test configs: retries and changes to which tests run are found", () => {
  found("tests/e2e/playwright.config.ts", PLAYWRIGHT, edit(PLAYWRIGHT, "retries: 0,", "retries: process.env.CI ? 2 : 0,"), /retrying/);
  found("tests/e2e/playwright.config.ts", PLAYWRIGHT, edit(PLAYWRIGHT, "  workers: 1,\n", '  workers: 1,\n  testIgnore: ["network.spec.ts"],\n'), /which tests run/);
  found("tests/e2e/playwright.config.ts", PLAYWRIGHT, edit(PLAYWRIGHT, "  forbidOnly: !!process.env.CI,\n", ""), /which tests run \(line 6 on the base is removed\)/);
  passes("tests/e2e/playwright.config.ts", PLAYWRIGHT, edit(PLAYWRIGHT, "retries: 0,", "retries:  0, // never"));
});

test("test configs: a new config passes, and in vite.config.ts only the test block counts", () => {
  passes("tests/e2e/playwright.config.ts", null, PLAYWRIGHT);
  const vite = `export default defineConfig({\n  plugins: [react()],\n  test: {\n    environment: "jsdom",\n    include: ["src/**/*.test.{ts,tsx}"],\n  },\n});\n`;
  found("vite.config.ts", vite, edit(vite, 'include: ["src/**/*.test.{ts,tsx}"]', 'include: ["src/viewer/**/*.test.{ts,tsx}"]'), /which tests run/);
  found("vite.config.ts", vite, edit(vite, '    environment: "jsdom",\n', '    environment: "jsdom",\n    retry: 2,\n'), /retrying/);
  passes("vite.config.ts", vite, edit(vite, "  plugins: [react()],\n", '  plugins: [react()],\n  optimizeDeps: { exclude: ["mupdf"] },\n'));
});

// --- Workflows -----------------------------------------------------------------------------------

const ci = (head) => problems(".github/workflows/ci.yml", WORKFLOW, head);
const ciFound = (head, message) => found(".github/workflows/ci.yml", WORKFLOW, head, message);
const ciPasses = (head) => passes(".github/workflows/ci.yml", WORKFLOW, head);

test("workflows: continue-on-error that is not false is found", () => {
  ciFound(edit(WORKFLOW, "        run: cargo test --workspace --locked\n", "        run: cargo test --workspace --locked\n        continue-on-error: true\n"), /step "Test" of job "rust"/);
  ciFound(edit(WORKFLOW, "  rust:\n", "  rust:\n    continue-on-error: ${{ matrix.experimental }}\n"), /job "rust"/);
  ciPasses(edit(WORKFLOW, "  rust:\n", "  rust:\n    continue-on-error: false\n"));
});

test("workflows: a new condition on a job or step that was there is found", () => {
  ciFound(edit(WORKFLOW, "      - name: Test\n", "      - name: Test\n        if: false\n"), /step "Test" of job "rust" gets a new condition/);
  ciFound(edit(WORKFLOW, "  guardrails:\n", "  guardrails:\n    if: ${{ false }}\n"), /job "guardrails" gets a new condition, so it may not run: if: false/);
  ciFound(edit(WORKFLOW, "if: needs.detect.outputs.rust == 'true'", "if: needs.detect.outputs.rust == 'false'"), /job "rust"/);
});

test("workflows: conditions that only run more, and new steps with a condition, pass", () => {
  ciPasses(edit(WORKFLOW, "if: failure()", "if: always()"));
  ciPasses(edit(WORKFLOW, "if: needs.detect.outputs.rust == 'true'", "if: needs.detect.outputs.rust == 'true' || github.event_name == 'workflow_dispatch'"));
  ciPasses(edit(WORKFLOW, "      - name: E2E tests\n", "      - name: Only on pull requests\n        if: github.event_name == 'pull_request'\n        run: echo hi\n      - name: E2E tests\n"));
});

test("workflows: a removed step or job is found, a renamed step passes", () => {
  const list = ciFound(edit(WORKFLOW, "      - name: Guard script tests\n        run: node --test scripts/ci/*.test.mjs\n", ""), /step "Guard script tests" of job "guardrails" is removed/);
  assert.ok(list.some((p) => /a guard script no longer runs/.test(p.message)));
  ciFound(edit(WORKFLOW, "  e2e:\n    runs-on: windows-latest\n    steps:\n      - name: E2E tests\n        run: pnpm e2e\n", ""), /job "e2e" is removed/);
  ciPasses(edit(WORKFLOW, "      - name: Test\n", "      - name: Rust tests\n"));
});

test("workflows: a guard may move to the base branch's copy, not back", () => {
  const moved = edit(WORKFLOW, "run: bash scripts/ci/check-forbidden.sh", "run: bash scripts/ci-base/check-forbidden.sh");
  ciPasses(moved);
  found(".github/workflows/ci.yml", moved, WORKFLOW, /a guard script no longer runs as on the base/);
});

test("workflows: a check that no longer runs, or whose failure is ignored, is found", () => {
  for (const run of [
    "run: echo skipped # cargo test --workspace --locked",
    "run: cargo test --workspace --locked || true",
    "run: cargo test --workspace --locked | tee test.log",
    "run: if cargo test --workspace --locked; then echo ok; fi",
    "run: cargo test --workspace --locked -- --skip render",
    "run: cargo test --workspace --locked --exclude pdf_worker",
  ]) {
    assert.ok(ci(edit(WORKFLOW, "run: cargo test --workspace --locked", run)).length > 0, run);
  }
  ciFound(edit(WORKFLOW, "        run: pnpm e2e\n", "        run: |\n          set +e\n          pnpm e2e\n"), /no longer stops the step/);
  ciFound(edit(WORKFLOW, "run: pnpm e2e", "run: pnpm e2e --retries=2"), /retrying/);
});

test("workflows: stricter or reworded checks pass", () => {
  ciPasses(edit(WORKFLOW, "run: cargo test --workspace --locked", "run: cargo test --workspace --locked --all-features"));
  ciPasses(edit(WORKFLOW, "run: cargo test --workspace --locked", "run: cargo test --workspace --locked -- --include-ignored"));
  ciPasses(edit(WORKFLOW, "        run: cargo test --workspace --locked\n", '        run: |\n          if ! cargo test --workspace --locked; then\n            echo "::error::see docs/workflow.md"\n            exit 1\n          fi\n'));
});

test("workflows: the pull request triggers may only gain paths", () => {
  ciPasses(edit(WORKFLOW, '      - "src/**"\n', '      - "src/**"\n      - "tests/**"\n'));
  for (const head of [
    edit(WORKFLOW, '    paths:\n      - "src/**"\n', '    paths:\n      - "src/viewer/**"\n'),
    edit(WORKFLOW, '  pull_request:\n    paths:\n      - "src/**"\n', "  pull_request:\n    types: [opened]\n    paths:\n      - \"src/**\"\n"),
    edit(WORKFLOW, '  pull_request:\n    paths:\n      - "src/**"\n', '  pull_request:\n    paths-ignore:\n      - "src/**"\n'),
    edit(WORKFLOW, '  pull_request:\n    paths:\n      - "src/**"\n', ""),
  ]) {
    ciFound(head, /when the workflow runs on pull requests/);
  }
  const inline = "name: X\non: [push, pull_request]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: pnpm lint\n";
  found(".github/workflows/x.yml", inline, edit(inline, "on: [push, pull_request]", "on: [push]"), /pull requests/);
});

test("workflows: the job whose outputs decide what runs is watched", () => {
  ciFound(edit(WORKFLOW, 'run: echo "rust=true"', 'run: echo "rust=false"'), /job "detect" decides whether "rust" run/);
});

test("workflows: a Dependabot bump of a pinned action, and a new workflow, pass", () => {
  ciPasses(WORKFLOW.replaceAll("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1", "actions/checkout@0123456789abcdef0123456789abcdef01234567 # v7.1.0"));
  passes(".github/workflows/new.yml", null, WORKFLOW);
});

test("workflows: every workflow of this repository parses, and is unchanged against itself", () => {
  const files = readdirSync(WORKFLOWS).filter((name) => /\.ya?ml$/.test(name));
  assert.ok(files.length > 0);
  for (const name of files) {
    const text = readFileSync(join(WORKFLOWS, name), "utf8");
    const jobs = parseWorkflow(text);
    assert.ok(jobs.length > 0, name);
    assert.ok(jobs.some((job) => job.steps.length > 0), name);
    passes(`.github/workflows/${name}`, text, text);
  }
  const guardrails = parseWorkflow(readFileSync(join(WORKFLOWS, "ci.yml"), "utf8")).find((job) => job.id === "guardrails");
  assert.ok(guardrails.text.includes("node scripts/ci-base/check-test-integrity.mjs --base HEAD^1"));
});

// --- Ban lists -----------------------------------------------------------------------------------

test("forbidden-patterns.txt: only a removed rule counts", () => {
  passes("scripts/ci/forbidden-patterns.txt", FORBIDDEN, edit(FORBIDDEN, "# Telemetry and crash reporting SDKs\n", ""));
  found("scripts/ci/forbidden-patterns.txt", FORBIDDEN, edit(FORBIDDEN, "posthog", "posthog-js"), /removed \(line 3 on the base\): posthog/);
});

const DENY = `[advisories]
yanked = "deny"
ignore = []

[bans]
deny = [
    # Network access of any kind (AGENTS.md principle 1, ADR 0009).
    { crate = "reqwest", reason = "HTTP client (ADR 0009)" },
]

[sources]
allow-git = ["https://github.com/winner0988/mupdf-rs"]
`;

test("deny.toml: a removed ban or advisory rule, or an ignored advisory, is found", () => {
  found("deny.toml", DENY, edit(DENY, '    { crate = "reqwest", reason = "HTTP client (ADR 0009)" },\n', ""), /\[bans\]/);
  found("deny.toml", DENY, edit(DENY, 'yanked = "deny"', 'yanked = "warn"'), /\[advisories\]/);
  found("deny.toml", DENY, edit(DENY, "ignore = []", 'ignore = ["RUSTSEC-2026-0001"]'), /ignore an advisory/);
  found("deny.toml", DENY, edit(DENY, "ignore = []", 'ignore = [\n    "RUSTSEC-2026-0001",\n]'), /ignore an advisory/);
});

test("deny.toml: a new ban, a comment and another source pass", () => {
  passes("deny.toml", DENY, edit(DENY, "]\n\n[sources]", '    { crate = "ureq", reason = "HTTP client (ADR 0009)" },\n]\n\n[sources]'));
  passes("deny.toml", DENY, edit(DENY, "    # Network access of any kind (AGENTS.md principle 1, ADR 0009).\n", "    # Network access (ADR 0009).\n"));
  passes("deny.toml", DENY, edit(DENY, "winner0988/mupdf-rs", "winner0988/mupdf-rs-2"));
});

const ESLINT = `export default defineConfig([
  globalIgnores(["dist", "target"]),
  {
    files: ["**/*.{ts,tsx}"],
    rules: {
      "no-restricted-globals": [
        "error",
        { name: "fetch", message: "The frontend must not make network requests." },
      ],
    },
  },
]);
`;

test("eslint.config.js: a ban turned off or narrowed, and files taken out of linting, are found", () => {
  for (const head of [
    edit(ESLINT, '        "error",\n', '        "off",\n'),
    edit(ESLINT, '        { name: "fetch", message: "The frontend must not make network requests." },\n', ""),
    edit(ESLINT, ']);\n', '  { files: ["src/net.ts"], rules: { "no-restricted-globals": "off" } },\n]);\n'),
    edit(ESLINT, 'globalIgnores(["dist", "target"])', 'globalIgnores(["dist", "target", "src/net"])'),
    edit(ESLINT, 'files: ["**/*.{ts,tsx}"]', 'files: ["src/ui/**/*.tsx"]'),
  ]) {
    assert.ok(problems("eslint.config.js", ESLINT, head).length > 0, head);
  }
  passes("eslint.config.js", ESLINT, edit(ESLINT, "]);\n", '  { files: ["src/components/ui/**/*.tsx"], rules: { "react-refresh/only-export-components": "off" } },\n]);\n'));
  passes("eslint.config.js", null, ESLINT);
});

test("scripts: a comment that turns the bans, or every rule, off is found", () => {
  const base = "export function load() {\n  return read();\n}\n";
  for (const comment of [
    "  // eslint-disable-next-line no-restricted-globals",
    "  /* eslint-disable */",
    "  // eslint-disable-line",
    '  /* eslint no-restricted-globals: "off" */',
  ]) {
    found("src/net.ts", base, edit(base, "  return read();\n", `${comment}\n  return read();\n`), /turns/);
  }
  passes("src/net.ts", base, edit(base, "  return read();\n", "  // eslint-disable-next-line react-hooks/exhaustive-deps -- runs once\n  return read();\n"));
});

test("package.json: a changed check script is found, other scripts pass", () => {
  const pkg = JSON.stringify({ scripts: { dev: "vite", lint: "eslint .", test: "vitest run", e2e: "playwright test" } }, null, 2);
  const change = (scripts) => JSON.stringify({ scripts }, null, 2);
  found("package.json", pkg, change({ dev: "vite", lint: "eslint .", test: "vitest run --passWithNoTests", e2e: "playwright test" }), /"test" script/);
  found("package.json", pkg, change({ dev: "vite", lint: "eslint .", test: "vitest run" }), /"e2e" script changes from "playwright test" to nothing/);
  passes("package.json", pkg, change({ dev: "vite --port 1421", lint: "eslint .", test: "vitest run", e2e: "playwright test" }));
});

test("gitleaks: a new marker that skips a finding is found, except in docs", () => {
  found(".env.example", "A=1\n", 'A=1\nTOKEN=abc123 # gitleaks:allow\n', /gitleaks/);
  found("src/key.ts", "export {};\n", 'export const KEY = "abc123"; // gitleaks:allow\n', /gitleaks/);
  found(".gitleaksignore", null, "abc123:src/key.ts:generic-api-key:1\n", /gitleaks/);
  passes("docs/workflow.md", "# CI\n", "# CI\n\nNever add `gitleaks:allow`.\n");
  passes(".env.example", "A=1 # gitleaks:allow\n", "B=2\nA=1 # gitleaks:allow\n");
});

// --- Files ---------------------------------------------------------------------------------------

test("a deleted test file, or one renamed out of the test runners' reach, is found", () => {
  found("src/a.test.ts", FRONTEND_TEST, null, /test file src\/a\.test\.ts is deleted/);
  found("crates/pdf_worker/tests/sandbox.rs", RUST, null, /is deleted/);
  found("tests/e2e/playwright.config.ts", PLAYWRIGHT, null, /test config tests\/e2e\/playwright\.config\.ts is deleted/);
  assert.match(problems("src/a.test.ts.bak", FRONTEND_TEST, FRONTEND_TEST, "src/a.test.ts")[0].message, /renamed to src\/a\.test\.ts\.bak/);
  assert.deepEqual(problems("src/viewer/a.test.ts", FRONTEND_TEST, FRONTEND_TEST, "src/a.test.ts"), []);
  passes("src/a.ts", "export {};\n", null);
});

test("this file's own tests may spell out every pattern", () => {
  passes("scripts/ci/check-test-integrity.test.mjs", "", 'it.skip("x");\n#[ignore]\n// eslint-disable\n');
});

test("checkChanges reports the problems of every file, in order", () => {
  const list = checkChanges([
    { path: "src/a.test.ts", oldPath: null, base: FRONTEND_TEST, head: null },
    { path: "src/b.ts", oldPath: null, base: "a\n", head: "b\n" },
    { path: "scripts/ci/forbidden-patterns.txt", oldPath: null, base: FORBIDDEN, head: "" },
  ]);
  assert.deepEqual(list.map((p) => p.path), ["src/a.test.ts", "scripts/ci/forbidden-patterns.txt", "scripts/ci/forbidden-patterns.txt"]);
});

// --- Command line --------------------------------------------------------------------------------

test("the command line compares two commits and reports with file and line", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "test-integrity-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  // A repository of its own, away from the user's and the system's git settings.
  const config = join(dir, "gitconfig");
  writeFileSync(config, "");
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: config,
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "test",
    GIT_AUTHOR_EMAIL: "test@example.invalid",
    GIT_COMMITTER_NAME: "test",
    GIT_COMMITTER_EMAIL: "test@example.invalid",
  };
  const repo = join(dir, "repo");
  mkdirSync(repo);
  const git = (...args) => execFileSync("git", args, { cwd: repo, env }).toString("utf8").trim();
  const write = (path, text) => {
    mkdirSync(dirname(join(repo, path)), { recursive: true });
    writeFileSync(join(repo, path), text);
  };
  const commit = (message) => {
    git("add", "-A");
    git("commit", "-q", "-m", message);
    return git("rev-parse", "HEAD");
  };
  const run = (...args) => spawnSync(process.execPath, [SCRIPT, ...args], { cwd: repo, env, encoding: "utf8" });

  git("-c", "init.defaultBranch=main", "init", "-q");
  write("src/a.test.ts", FRONTEND_TEST);
  write("README.md", "# Test\n");
  const base = commit("base");

  write("src/a.test.ts", edit(FRONTEND_TEST, '  it("never calls fetch"', '  it.skip("never calls fetch"'));
  write("README.md", "# Test\n\nMore.\n");
  const skipped = commit("skip a test");
  let result = run("--base", base);
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.match(result.stdout, /^::error file=src\/a\.test\.ts,line=4::this keeps tests from running/m);
  assert.match(result.stdout, /^::error::This PR switches off a check that would judge it/m);

  git("mv", "src/a.test.ts", "src/b.test.ts");
  write("src/c.test.ts", 'it("adds", () => {\n  expect(1 + 1).toBe(2);\n});\n');
  commit("move the test and add one");
  result = run("--base", skipped, "--head", "HEAD");
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /^OK: none of the 2 changed files switches a check off\.$/m);

  // No origin/main to take the merge base from, and an unknown argument.
  assert.equal(run().status, 2);
  assert.equal(run("--base").status, 2);
  assert.equal(run("--bogus", "x").status, 2);
});
