#!/usr/bin/env node
// Fails if a PR switches off a check that judges it (#630). AGENTS.md forbids skipping,
// disabling or deleting tests and CI checks to turn CI green; this compares the PR with its
// base and reports, with file and line:
//   - skips: #[ignore], an always-false #[cfg(any())], or #[should_panic] on an existing test
//     in Rust; .skip( .only( .fixme( .todo( .fail( and the like in test files; a test file
//     deleted, or renamed to a name no test runner picks up;
//   - retries above 0, and changes to which tests run (test configs, workflows, the check
//     scripts in package.json);
//   - workflows: a job or step removed, a new or changed if: on a job or step that was there
//     before, continue-on-error that is not false, a change to when the workflow runs or to a
//     job whose outputs decide whether other jobs run, a guard script or check command that
//     no longer runs as before, or whose failure is ignored;
//   - ban lists: a line removed from scripts/ci/forbidden-patterns.txt, from deny.toml's
//     [advisories] or [bans], or from the no-restricted-* rules in eslint.config.js; an
//     advisory ignored or a source allowed in deny.toml; a no-restricted rule turned off, or
//     files taken out of linting; a comment that turns those rules, or all rules, off for some
//     code; a finding that gitleaks is told to skip.
// When one is really needed, the owner says why in the PR and merges it (ruleset bypass).
//
// Usage: node scripts/ci/check-test-integrity.mjs [--base <rev>] [--head <rev>]
//   Compares <head> (by default HEAD) with <base>, by default the merge base of HEAD and
//   origin/main. CI runs the base branch's copy of this script (scripts/ci-base/) with
//   --base HEAD^1, the base parent of the PR's merge commit, so a PR cannot change the rules
//   that judge it.
// Tests: node --test scripts/ci/*.test.mjs

import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// This file's own tests spell out every pattern they must find.
const OWN_TESTS = "scripts/ci/check-test-integrity.test.mjs";

const isRust = (path) => path.endsWith(".rs");
const isScript = (path) => /\.[cm]?[jt]sx?$/.test(path);
const isViteConfig = (path) => /(?:^|\/)vite\.config\.[cm]?[jt]s$/.test(path);
const isTestConfig = (path) =>
  /(?:^|\/)(?:playwright|vitest)\.config\.[cm]?[jt]s$/.test(path) ||
  isViteConfig(path) ||
  path.endsWith(".config/nextest.toml");
const isTestScript = (path) =>
  isScript(path) &&
  !isTestConfig(path) &&
  (/\.(?:test|spec)\.[cm]?[jt]sx?$/.test(path) || path.startsWith("tests/e2e/"));
const isRustTest = (path) =>
  isRust(path) && (/(?:^|\/)tests\//.test(path) || path.startsWith("fuzz/fuzz_targets/"));
const isTestFile = (path) => isTestScript(path) || isRustTest(path) || isTestConfig(path);
const isWorkflow = (path) => /^\.github\/workflows\/[^/]+\.ya?ml$/.test(path);
const isEslintConfig = (path) => /^eslint\.config\.[cm]?js$/.test(path);
const isGitleaksConfig = (path) => /(?:^|\/)\.?gitleaks(?:\.toml|ignore)$/.test(path);
const isDoc = (path) => path.startsWith("docs/") || path.endsWith(".md");
const FORBIDDEN_PATTERNS = "scripts/ci/forbidden-patterns.txt";

/** Whether a rule needs this file's line diff; every other text file is only searched for the gitleaks marker. */
const isDiffed = (path) =>
  isRust(path) ||
  isScript(path) ||
  isTestConfig(path) ||
  isWorkflow(path) ||
  isGitleaksConfig(path) ||
  ["package.json", "deny.toml", FORBIDDEN_PATTERNS].includes(path);

// --- Line diff -------------------------------------------------------------------------------

/** Above this many changed lines in one file, every line between the common start and end counts as changed. */
const MAX_EDITS = 4000;

/**
 * Myers' line diff: the 1-based numbers of the lines of `a` that are removed and of the lines
 * of `b` that are added.
 */
export function diffLines(a, b) {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const n = endA - start;
  const m = endB - start;
  const removed = [];
  const added = [];
  const everything = () => {
    for (let i = start; i < endA; i++) removed.push(i + 1);
    for (let j = start; j < endB; j++) added.push(j + 1);
    return { removed, added };
  };
  if (n === 0 || m === 0) return everything();

  const max = n + m;
  const offset = max + 1;
  const v = new Int32Array(2 * max + 3);
  const trace = [];
  let done = false;
  for (let d = 0; d <= max && !done; d++) {
    if (d > MAX_EDITS) return everything();
    // v as it was before step d, for k from -d - 1 to d + 1.
    trace.push(v.slice(offset - d - 1, offset + d + 2));
    for (let k = -d; k <= d; k += 2) {
      let x =
        k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1]) ? v[offset + k + 1] : v[offset + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[start + x] === b[start + y]) {
        x++;
        y++;
      }
      v[offset + k] = x;
      if (x >= n && y >= m) {
        done = true;
        break;
      }
    }
  }

  let x = n;
  let y = m;
  for (let d = trace.length - 1; d > 0; d--) {
    const before = trace[d];
    const at = (k) => before[k + d + 1];
    const k = x - y;
    const down = k === -d || (k !== d && at(k - 1) < at(k + 1));
    const prevK = down ? k + 1 : k - 1;
    const prevX = at(prevK);
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      x--;
      y--;
    }
    if (down) added.push(start + prevY + 1);
    else removed.push(start + prevX + 1);
    x = prevX;
    y = prevY;
  }
  removed.sort((p, q) => p - q);
  added.sort((p, q) => p - q);
  return { removed, added };
}

// --- Workflow files (the block-style YAML that GitHub Actions workflows use) ------------------

/** Indentation of a line, or -1 for a blank line or a comment. */
function indentOf(line) {
  const match = /^( *)(\S)/.exec(line);
  return !match || match[2] === "#" ? -1 : match[1].length;
}

/** A YAML value without its trailing comment. */
function withoutComment(value) {
  let quote = null;
  for (let i = 0; i < value.length; i++) {
    const c = value[i];
    if (quote) {
      if (c === quote) quote = null;
    } else if (c === "'" || c === '"') {
      quote = c;
    } else if (c === "#" && (i === 0 || /\s/.test(value[i - 1]))) {
      return value.slice(0, i);
    }
  }
  return value;
}

/** A condition or value as GitHub reads it: no comment, quotes, ${{ }} or extra spaces. */
function normalize(value) {
  let text = withoutComment(value).trim();
  if (text.length >= 2 && (text[0] === "'" || text[0] === '"') && text.at(-1) === text[0]) text = text.slice(1, -1);
  const expression = /^\$\{\{([\s\S]*)\}\}$/.exec(text);
  if (expression) text = expression[1];
  return text.replace(/\s+/g, " ").trim();
}

/** The `key: value` entries at the indentation of the first line of lines[from..to). */
function entries(lines, from, to) {
  const found = [];
  let indent = -1;
  for (let i = from; i < to; i++) {
    const own = indentOf(lines[i]);
    if (own < 0) continue;
    if (indent < 0) indent = own;
    if (own < indent) break;
    if (own > indent) continue;
    const match = /^\s*([^\s:#'"-][^:]*?|"[^"]*"|'[^']*')\s*:(?:\s+(.*))?$/.exec(lines[i]);
    if (match) found.push({ key: match[1].replace(/^["']|["']$/g, ""), raw: match[2] ?? "", index: i, to });
  }
  for (let e = 0; e + 1 < found.length; e++) found[e].to = found[e + 1].index;
  return found;
}

/** The value of an entry: inline, or the lines of a block scalar (run: |). */
function valueOf(lines, entry) {
  if (!entry) return null;
  const raw = withoutComment(entry.raw).trim();
  if (/^[|>][+-]?\d*[+-]?$/.test(raw)) {
    return lines
      .slice(entry.index + 1, entry.to)
      .map((line) => line.trim())
      .filter(Boolean)
      .join("\n");
  }
  return normalize(raw);
}

/** `uses: owner/action@ref # vX` without the ref, which Dependabot changes. */
const withoutRef = (line) => line.replace(/(uses:\s*["']?[^@\s"']+)@[^\s"']+["']?(\s+#.*)?$/, "$1");

function parseStep(lines, from, to) {
  const own = lines.slice(from, to);
  own[0] = own[0].replace(/^(\s*)-/, "$1 ");
  const props = new Map(entries(own, 0, own.length).map((e) => [e.key, { ...e, line: from + e.index + 1 }]));
  const value = (key) => valueOf(own, props.get(key));
  const name = value("name");
  const uses = value("uses");
  const run = value("run");
  const key = name ?? (uses ? uses.replace(/@.*$/, "") : null) ?? (run ? run.split("\n")[0] : null) ?? value("id");
  const named = props.get("name");
  // What the step does: its lines without the name, comments, blank lines and action refs.
  const body = own
    .filter((line, i) => indentOf(line) >= 0 && !(named && i >= named.index && i < named.to))
    .map((line) => withoutRef(line).trim())
    .join("\n");
  return {
    key: key ?? `step at line ${from + 1}`,
    line: from + 1,
    if: value("if"),
    ifLine: props.get("if")?.line ?? from + 1,
    continueOnError: value("continue-on-error"),
    continueOnErrorLine: props.get("continue-on-error")?.line ?? from + 1,
    body,
  };
}

/** Jobs of a workflow file, each with its condition, continue-on-error, steps and text. */
export function parseWorkflow(text) {
  const lines = text.split(/\r?\n/);
  const top = entries(lines, 0, lines.length);
  const section = top.find((e) => e.key === "jobs");
  if (!section) return [];
  return entries(lines, section.index + 1, section.to).map((job) => {
    const props = new Map(entries(lines, job.index + 1, job.to).map((e) => [e.key, e]));
    const steps = [];
    const list = props.get("steps");
    if (list) {
      let indent = -1;
      const starts = [];
      for (let i = list.index + 1; i < list.to; i++) {
        const own = indentOf(lines[i]);
        if (own < 0) continue;
        if (indent < 0) indent = own;
        if (own === indent && /^\s*-(\s|$)/.test(lines[i])) starts.push(i);
      }
      starts.forEach((from, s) => steps.push(parseStep(lines, from, starts[s + 1] ?? list.to)));
    }
    return {
      id: job.key,
      line: job.index + 1,
      if: valueOf(lines, props.get("if")),
      ifLine: (props.get("if")?.index ?? job.index) + 1,
      continueOnError: valueOf(lines, props.get("continue-on-error")),
      continueOnErrorLine: (props.get("continue-on-error")?.index ?? job.index) + 1,
      steps,
      text: lines.slice(job.index, job.to).map(withoutRef).join("\n").trimEnd(),
    };
  });
}

// --- Rules -------------------------------------------------------------------------------------

const SKIPS_IN_RUST = [
  /#!?\[\s*ignore\b/,
  /#!?\[\s*cfg_attr\s*\(.*\bignore\b/,
  // cfg(any()) is always false: the item is not compiled at all.
  /#!?\[\s*cfg\s*\(\s*any\s*\(\s*\)\s*\)\s*\]/,
];

// A test or group skipped, focused or expected to fail through the test API: test, it,
// describe..., Playwright's testInfo, node:test's t, vitest's ctx. Matched on code whose string
// literals are emptied, so a test title cannot match.
const SKIP_CALL =
  /\b(?:it|test|describe|suite|context|bench|testInfo|t|ctx)(?:\.\w+)*\.(skip|only|fixme|todo|fails?|skipIf|runIf)\s*\(/g;
// Kinds that may take a condition first, as in Playwright's test.skip(condition, reason).
const CONDITIONAL = new Set(["skip", "fixme", "fail"]);
const SKIP_ALIAS = /^\s*(?:xit|xtest|xdescribe|fit|fdescribe)\s*\(/;
const SKIP_OPTION = /\b(?:skip|only|todo|fails)\s*:\s*(?:true\b|["'`])/;
const TEST_DECLARATION = /^\s*(?:it|test|describe|suite|context)(?:\.\w+)*\s*\(/;

// Retries above 0 in a test config or workflow. The spaces are inside the lookahead, so the
// match cannot back off a space to get past it.
const RETRIES = [
  /\bretries\s*[:=](?!\s*0\s*(?:[,}\]]|$|\/\/|#))/,
  /\bretry\s*:(?!\s*0\s*(?:[,}\]]|$|\/\/))/,
  /--retr(?:y|ies)(?:=|\s+)(?!\s*0\b)/,
];
// In test code only a count counts (a prop called retry is common).
const RETRIES_IN_TESTS = /\bretr(?:y|ies)\s*:\s*(?:[1-9]|[^,}]*\?\s*[1-9])/;

/** A line of JavaScript with the contents of its string literals removed (the quotes stay). */
const withoutStrings = (text) =>
  text.replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g, (literal) => literal[0] + literal[0]);
const isCodeComment = (code) => /^\s*(?:\/\/|\/\*|\*)/.test(code);

const TEST_SELECTION_KEYS =
  /\b(?:include|exclude|testMatch|testIgnore|testDir|grep|grepInvert|forbidOnly|allowOnly|passWithNoTests|testNamePattern)\b/;

const TEST_SELECTION_FLAGS =
  /(?:^|\s)--(?:skip|exclude|grep|grep-invert|ignored|pass-with-no-tests|passWithNoTests|test-name-pattern|testNamePattern|last-failed|only-changed)\b/;

const CHECK_COMMAND =
  /\b(?:cargo\s+(?:\+\S+\s+)?(?:test|nextest|clippy|fmt|deny|fuzz|check)\b|pnpm\s+(?:run\s+)?(?:test|e2e|lint|typecheck|audit)\b|node\s+--test\b|vitest\b|playwright\s+test\b|eslint\b|tsc\b|gitleaks\b|tests\/corpus\/generate\.py)/;

// `|| true` and the like after a check, or a pipe, whose exit status replaces the check's
// (GitHub's default bash runs without pipefail).
const SWALLOWS_FAILURE = /\|\|\s*(?:true|:|exit\s+0|echo)\b|;\s*exit\s+0\b|(?<!\|)\|(?!\|)/;
// A shell condition, whose failure does not fail the step: `if cargo test; then` (not `if !`).
const TESTED_BY_SHELL = /\b(?:if|while|until)\s+$/;

const CHECK_SCRIPTS = ["lint", "typecheck", "test", "e2e"];

// A condition that can only make a step or job run more often.
const RUNS_MORE = /^(?:always\(\)|!\s*cancelled\(\))$/;
// A condition with which a step or job runs even after an earlier one failed.
const RUNS_AFTER_FAILURE = /(?:\balways|!\s*cancelled)\s*\(\s*\)/;

/** The 1-based line range of the bracketed block that starts on line index `i`. */
function blockAt(lines, i, comment) {
  let depth = 0;
  for (let j = i; j < lines.length; j++) {
    const code = lines[j]
      .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`/g, "")
      .replace(comment, "");
    depth += (code.match(/[[{(]/g) ?? []).length - (code.match(/[\]})]/g) ?? []).length;
    if (depth <= 0) return [i + 1, j + 1];
  }
  return [i + 1, lines.length];
}

const JS_COMMENT = /\/\/.*$|\/\*.*?\*\//g;
const TOML_COMMENT = /#.*$/;

/** The 1-based line ranges, in JavaScript `lines`, of the blocks that start where `opens` matches. */
function blocks(lines, opens) {
  const found = [];
  lines.forEach((line, i) => {
    if (opens.test(line)) found.push(blockAt(lines, i, JS_COMMENT));
  });
  return found;
}

/** The 1-based line ranges of the TOML arrays `key = [...]` named in `wanted` ({ section: [keys] }). */
function tomlArrays(lines, wanted) {
  const found = [];
  let section = null;
  lines.forEach((line, i) => {
    const header = /^\s*\[([^\]]+)\]/.exec(line);
    if (header) section = header[1].trim();
    const key = /^\s*([\w-]+)\s*=\s*\[/.exec(line);
    if (key && wanted[section]?.includes(key[1])) found.push(blockAt(lines, i, TOML_COMMENT));
  });
  return found;
}

/** The TOML section each line belongs to (null before the first header). */
function tomlSections(lines) {
  let section = null;
  return lines.map((line) => {
    const header = /^\s*\[([^\]]+)\]/.exec(line);
    if (header) section = header[1].trim();
    return section;
  });
}

const inside = (ranges, line) => ranges.some(([from, to]) => line >= from && line <= to);
const isComment = (text, prefix) => text.trim() === "" || text.trim().startsWith(prefix);

/**
 * Whether the test or group around 1-based `line` is new in the PR: its declaration is added and
 * so are most of its lines (a diff may pair a closing line with an old one). At the top level,
 * whether the file is new.
 */
function inNewTest(lines, addedSet, line, newFile) {
  const indent = lines[line - 1].search(/\S/);
  for (let i = line - 2; i >= 0; i--) {
    const own = lines[i].search(/\S/);
    if (own < 0 || own >= indent || !TEST_DECLARATION.test(withoutStrings(lines[i]))) continue;
    if (!addedSet.has(i + 1)) return false;
    const [from, to] = blockAt(lines, i, JS_COMMENT);
    let total = 0;
    let added = 0;
    for (let n = from; n <= to; n++) {
      if (lines[n - 1].trim() === "") continue;
      total++;
      if (addedSet.has(n)) added++;
    }
    return added >= 0.8 * total;
  }
  return newFile;
}

/** The skip-like calls on added line `line` of a test file; a conditional skip in a new test is allowed. */
function skipsOn(lines, addedSet, line, newFile) {
  const code = withoutStrings(lines[line - 1]);
  if (isCodeComment(code)) return [];
  const found = [];
  for (const match of code.matchAll(SKIP_CALL)) {
    const kind = match[1];
    if (CONDITIONAL.has(kind)) {
      // The first argument, which may start on the next line.
      let first = code.slice(match.index + match[0].length).trim();
      for (let i = line; first === "" && i < Math.min(lines.length, line + 3); i++) first = withoutStrings(lines[i]).trim();
      const conditional = first !== "" && !/^["'`)]/.test(first);
      if (conditional && inNewTest(lines, addedSet, line, newFile)) continue;
    }
    found.push(kind);
  }
  if (SKIP_ALIAS.test(code) || SKIP_OPTION.test(code)) found.push("skip");
  return found;
}

/** Whether a workflow line runs a check whose failure it then ignores. */
function swallowsFailure(text) {
  const check = CHECK_COMMAND.exec(text);
  return (
    check !== null &&
    (SWALLOWS_FAILURE.test(text.slice(check.index)) || TESTED_BY_SHELL.test(text.slice(0, check.index)))
  );
}

/**
 * Problems in one changed file: { path, line (in the PR's version, or null), message }.
 * `base` and `head` are the file's text on the base and in the PR, null when it is missing or
 * binary; `oldPath` is the file's path on the base when the PR renames it.
 */
export function checkChange({ path, oldPath = null, base, head }) {
  const problems = [];
  const report = (line, message) => problems.push({ path, line, message });
  const baseLines = base === null ? [] : base.split(/\r?\n/);
  const headLines = head === null ? [] : head.split(/\r?\n/);
  const { removed, added } = diffLines(baseLines, headLines);
  const addedSet = new Set(added);
  const addedLines = added.map((line) => ({ line, text: headLines[line - 1] }));
  const removedLines = removed.map((line) => ({ line, text: baseLines[line - 1] }));
  const own = path === OWN_TESTS;

  if (head === null && base !== null && isTestFile(path)) {
    report(null, `${isTestConfig(path) ? "test config" : "test file"} ${path} is deleted.`);
  }
  if (oldPath !== null && oldPath !== path && isTestFile(oldPath) && !isTestFile(path)) {
    report(null, `test file ${oldPath} is renamed to ${path}, where no test runner picks it up.`);
  }
  if (!isDiffed(path)) {
    // Only the gitleaks marker applies here, and needs no line diff.
    const before = new Set(baseLines);
    headLines.forEach((text, i) => {
      if (!isDoc(path) && /gitleaks[:]allow/.test(text) && !before.has(text)) {
        report(i + 1, `this tells gitleaks to skip a finding: ${text.trim()}`);
      }
    });
    return problems;
  }

  // In vite.config.*, only the `test:` block decides which tests run.
  const testBlock = (lines) => (isViteConfig(path) ? blocks(lines, /^\s*test\s*:\s*\{/) : [[1, lines.length]]);
  const headTestBlock = testBlock(headLines);
  const baseTestBlock = testBlock(baseLines);
  const eslintBans = isEslintConfig(path) ? blocks(headLines, /["']no-restricted-[\w-]+["']\s*:/) : [];
  const eslintIgnores = isEslintConfig(path) && base !== null ? blocks(headLines, /\bglobalIgnores\s*\(|\bignores\s*:/) : [];
  const advisoryIgnores = path === "deny.toml" ? tomlArrays(headLines, { advisories: ["ignore"] }) : [];

  for (const { line, text } of own ? [] : addedLines) {
    if (isRust(path) && SKIPS_IN_RUST.some((p) => p.test(text))) {
      report(line, `this keeps Rust code or tests from running: ${text.trim()}`);
    }
    if (isRust(path) && /#\s*\[\s*should_panic\b/.test(text)) {
      const fn = headLines.findIndex((l, i) => i >= line && /\bfn\s+\w+/.test(l));
      if (fn >= 0 && !addedSet.has(fn + 1)) {
        report(line, `#[should_panic] makes an existing Rust test pass when it fails: ${text.trim()}`);
      }
    }
    if (isTestScript(path)) {
      const kinds = skipsOn(headLines, addedSet, line, base === null);
      if (kinds.length > 0) {
        report(line, `this keeps tests from running, or passes them when they fail (${kinds.join(", ")}): ${text.trim()}`);
      }
      const code = withoutStrings(text);
      if (!isCodeComment(code) && RETRIES_IN_TESTS.test(code)) {
        report(line, `retrying lets a failing test pass on a later try: ${text.trim()}`);
      }
    }
    if ((isWorkflow(path) || (isTestConfig(path) && inside(headTestBlock, line))) && RETRIES.some((p) => p.test(text))) {
      report(line, `retrying lets a failing test pass on a later try: ${text.trim()}`);
    }
    if (isWorkflow(path) && TEST_SELECTION_FLAGS.test(text)) {
      report(line, `this changes which tests or checks run: ${text.trim()}`);
    }
    if (isWorkflow(path) && swallowsFailure(text)) {
      report(line, `this lets a failing check pass: ${text.trim()}`);
    }
    if (isWorkflow(path) && /^\s*set\s+(?:\+e\b|\+o\s+errexit\b)/.test(text)) {
      report(line, `a failing command no longer stops the step: ${text.trim()}`);
    }
    if (isScript(path)) {
      const comment = /(?:\/\/|\/\*)\s*eslint-disable(?:-next-line|-line)?(?=\s|\*\/|$)(.*)/.exec(text);
      if (comment) {
        const rules = comment[1].replace(/\*\/.*$/, "").split("--")[0].trim();
        if (rules === "" || /no-restricted-/.test(rules)) {
          report(line, `this turns off ${rules === "" ? "every lint rule" : rules} for some code: ${text.trim()}`);
        }
      }
      if (/\/\*\s*eslint\s[^*]*no-restricted-[\w-]+["']?\s*:\s*\[?\s*["']?(?:off|warn|0|1)\b/.test(text)) {
        report(line, `this turns a ban off for this file: ${text.trim()}`);
      }
    }
    if (
      isEslintConfig(path) &&
      (/no-restricted-[\w-]+["']?\s*:\s*\[?\s*["']?(?:off|warn|0|1)\b/.test(text) ||
        (inside(eslintBans, line) && /^\s*\[?\s*(?:["'](?:off|warn)["']|[01])\s*[,\]]?\s*$/.test(text)))
    ) {
      report(line, `this turns a ban off: ${text.trim()}`);
    }
    if (isEslintConfig(path) && inside(eslintIgnores, line) && !isComment(text, "//")) {
      report(line, `this takes files out of linting: ${text.trim()}`);
    }
    if (inside(advisoryIgnores, line) && !isComment(text, "#") && !/^\s*[\w-]+\s*=\s*\[\s*\]\s*$/.test(text)) {
      report(line, `cargo-deny is told to ignore an advisory: ${text.trim()}`);
    }
    // Docs may name the marker; [:] keeps this line from matching itself.
    if ((/gitleaks[:]allow/.test(text) && !isDoc(path)) || (isGitleaksConfig(path) && !isComment(text, "#"))) {
      report(line, `this tells gitleaks to skip a finding: ${text.trim()}`);
    }
  }

  // A new test config sets up which tests run for the first time; only a change counts.
  if (isTestConfig(path) && base !== null) {
    for (const { line, text } of addedLines) {
      if (inside(headTestBlock, line) && TEST_SELECTION_KEYS.test(text)) {
        report(line, `this changes which tests run: ${text.trim()}`);
      }
    }
    for (const { line, text } of removedLines) {
      if (inside(baseTestBlock, line) && TEST_SELECTION_KEYS.test(text)) {
        report(null, `this changes which tests run (line ${line} on the base is removed): ${text.trim()}`);
      }
    }
  }

  if (path === FORBIDDEN_PATTERNS) {
    for (const { line, text } of removedLines) {
      if (!isComment(text, "#")) report(null, `a forbidden pattern is removed (line ${line} on the base): ${text.trim()}`);
    }
  }

  if (path === "deny.toml") {
    const sections = tomlSections(baseLines);
    for (const { line, text } of removedLines) {
      if (["advisories", "bans"].includes(sections[line - 1]) && !isComment(text, "#")) {
        report(null, `a cargo-deny rule is removed (line ${line} on the base, [${sections[line - 1]}]): ${text.trim()}`);
      }
    }
  }

  if (isEslintConfig(path)) {
    const bans = blocks(baseLines, /["']no-restricted-[\w-]+["']\s*:/);
    for (const { line, text } of removedLines) {
      if (inside(bans, line) && !isComment(text, "//")) {
        report(null, `a no-restricted rule is removed (line ${line} on the base): ${text.trim()}`);
      } else if (/\bfiles\s*:/.test(text)) {
        report(null, `the files a lint config applies to change (line ${line} on the base): ${text.trim()}`);
      }
    }
  }

  if (path === "package.json" && base !== null) {
    const scripts = (text) => {
      try {
        return JSON.parse(text).scripts ?? {};
      } catch {
        return null;
      }
    };
    const before = scripts(base);
    const after = head === null ? {} : scripts(head);
    if (before && after) {
      for (const name of CHECK_SCRIPTS) {
        if (name in before && before[name] !== after[name]) {
          const line = headLines.findIndex((l) => l.includes(`"${name}":`)) + 1 || null;
          report(line, `the "${name}" script changes from "${before[name]}" to ${after[name] === undefined ? "nothing" : `"${after[name]}"`}.`);
        }
      }
    }
  }

  if (isWorkflow(path)) problems.push(...checkWorkflow(path, base, head, removedLines));
  return problems;
}

function checkWorkflow(path, base, head, removedLines) {
  const problems = [];
  const report = (line, message) => problems.push({ path, line, message });
  const before = base === null ? [] : parseWorkflow(base);
  const after = head === null ? [] : parseWorkflow(head);
  const afterById = new Map(after.map((job) => [job.id, job]));
  const isOn = (value) => value !== null && value !== "false";

  // A condition that is new, unless it can only make the job or step run more often: always(),
  // !cancelled(), or the old condition `|| something`.
  const newCondition = (now, was) =>
    now !== null &&
    now !== was &&
    !RUNS_MORE.test(now) &&
    !(was !== null && (now.startsWith(`${was} ||`) || now.startsWith(`(${was}) ||`)));
  // A condition that let the job or step run after an earlier failure, and is gone: it now runs
  // only when everything before it passed, so an expected red can hide its findings.
  const lostRunAnyway = (now, was) => now === null && was !== null && RUNS_AFTER_FAILURE.test(was);
  const runsCheck = (step) => /scripts\/ci(?:-base)?\//.test(step.body) || CHECK_COMMAND.test(step.body);

  // The pull request triggers may gain paths or branches to run on, nothing else.
  const was = base === null ? null : prTriggers(base);
  if (was !== null) {
    const now = (head === null ? null : prTriggers(head)) ?? [];
    const lost = was.filter((t) => !now.includes(t));
    const gained = now.filter((t) => !was.includes(t) && !/^[\w-]+\.(?:paths|branches|tags)\[\] /.test(t));
    if (lost.length > 0 || gained.length > 0) {
      report(null, `this changes when the workflow runs on pull requests: ${[...lost.map((t) => `- ${t}`), ...gained.map((t) => `+ ${t}`)].join("; ")}`);
    }
  }

  for (const job of after) {
    const old = before.find((j) => j.id === job.id);
    if (isOn(job.continueOnError) && job.continueOnError !== old?.continueOnError) {
      report(job.continueOnErrorLine, `continue-on-error: ${job.continueOnError} lets job "${job.id}" fail without failing CI.`);
    }
    // Steps pair up by name (or action, or first command); a renamed step by what it does.
    const oldSteps = old ? [...old.steps] : [];
    const take = (match) => {
      const index = oldSteps.findIndex(match);
      return index >= 0 ? oldSteps.splice(index, 1)[0] : null;
    };
    const pairs = job.steps.map((step) => take((s) => s.key === step.key));
    job.steps.forEach((step, s) => {
      pairs[s] ??= take((o) => o.body === step.body);
    });
    job.steps.forEach((step, s) => {
      const was = pairs[s];
      if (isOn(step.continueOnError) && step.continueOnError !== was?.continueOnError) {
        report(step.continueOnErrorLine, `continue-on-error: ${step.continueOnError} lets step "${step.key}" of job "${job.id}" fail without failing CI.`);
      }
      if (was && newCondition(step.if, was.if)) {
        report(step.ifLine, `step "${step.key}" of job "${job.id}" gets a new condition, so it may not run: if: ${step.if}`);
      }
      if (was && lostRunAnyway(step.if, was.if) && runsCheck(step)) {
        report(step.line, `step "${step.key}" of job "${job.id}" no longer runs after an earlier step fails: its if: ${was.if} is removed.`);
      }
    });
    for (const gone of oldSteps) {
      report(job.line, `step "${gone.key}" of job "${job.id}" is removed.`);
    }
    if (old && newCondition(job.if, old.if)) {
      report(job.ifLine, `job "${job.id}" gets a new condition, so it may not run: if: ${job.if}`);
    }
    if (old && lostRunAnyway(job.if, old.if)) {
      report(job.line, `job "${job.id}" no longer runs after a job it needs fails: its if: ${old.if} is removed.`);
    }
  }
  for (const job of before) {
    if (!afterById.has(job.id)) report(null, `job "${job.id}" is removed.`);
  }

  // A job whose outputs decide whether other jobs run (if: needs.<job>.outputs...) gates them.
  const gated = new Map();
  for (const job of after) {
    for (const match of (job.if ?? "").matchAll(/needs\.([\w-]+)\.outputs/g)) {
      gated.set(match[1], [...(gated.get(match[1]) ?? []), job.id]);
    }
  }
  for (const [id, jobs] of gated) {
    const old = before.find((j) => j.id === id);
    const now = afterById.get(id);
    if (old && now && old.text !== now.text) {
      report(now.line, `job "${id}" decides whether ${[...new Set(jobs)].map((j) => `"${j}"`).join(", ")} run, and it changes.`);
    }
  }

  // Every guard script and check command of a removed line still runs somewhere in the file, word
  // for word, on a line that is not a comment. A guard may move to the base branch's copy
  // (scripts/ci-base/), not back from it.
  const live = (head ?? "")
    .split(/\r?\n/)
    .filter((line) => indentOf(line) >= 0)
    .map((line) => withoutComment(line).trim());
  const runs = (command) =>
    live.some((line) => line.includes(command) || line.includes(command.replaceAll("scripts/ci/", "scripts/ci-base/")));
  for (const { line, text } of removedLines) {
    if (indentOf(text) < 0) continue;
    const code = withoutComment(text).trim();
    const guard = /scripts\/ci(?:-base)?\//.exec(code);
    const check = CHECK_COMMAND.exec(code);
    if (!guard && !check) continue;
    const command = code.slice(guard ? guard.index : check.index).replace(/\s*(?:;\s*then|\\)$/, "");
    if (runs(command)) continue;
    report(null, `${guard ? "a guard script" : "a check"} no longer runs as on the base (line ${line} there): ${code}`);
  }
  return problems;
}

/**
 * The pull request triggers of a workflow, a line each ("pull_request.paths[] src/**"), or null
 * when it has none.
 */
function prTriggers(text) {
  const lines = text.split(/\r?\n/);
  const on = entries(lines, 0, lines.length).find((e) => e.key === "on");
  if (!on) return null;
  const inline = normalize(on.raw);
  const EVENTS = /^(?:pull_request|pull_request_target|merge_group)$/;
  if (inline !== "") return inline.split(/[\s,[\]]+/).some((event) => EVENTS.test(event)) ? [inline] : null;
  const found = [];
  for (const event of entries(lines, on.index + 1, on.to)) {
    if (!EVENTS.test(event.key)) continue;
    found.push(`${event.key}: ${normalize(event.raw)}`);
    for (const filter of entries(lines, event.index + 1, event.to)) {
      found.push(`${event.key}.${filter.key}: ${normalize(filter.raw)}`);
      for (let i = filter.index + 1; i < filter.to; i++) {
        if (indentOf(lines[i]) >= 0) found.push(`${event.key}.${filter.key}[] ${normalize(lines[i].trim().replace(/^-\s*/, ""))}`);
      }
    }
  }
  return found.length > 0 ? found : null;
}

/** Problems in all changed files, in order. */
export function checkChanges(changes) {
  return changes.flatMap(checkChange);
}

// --- Command line ----------------------------------------------------------------------------

function git(args, input) {
  return execFileSync("git", args, { input, maxBuffer: 1 << 30 });
}

/** The changed files between `base` and `head`, with renames followed. */
export function changedFiles(base, head = "HEAD") {
  const fields = git(["diff", "--name-status", "-z", "-M", base, head, "--"]).toString("utf8").split("\0");
  const changes = [];
  for (let i = 0; i < fields.length - 1; ) {
    const status = fields[i++];
    if (status.startsWith("R") || status.startsWith("C")) {
      const oldPath = fields[i++];
      const path = fields[i++];
      changes.push({ status: status[0], oldPath: status[0] === "R" ? oldPath : null, path });
    } else {
      changes.push({ status: status[0], oldPath: null, path: fields[i++] });
    }
  }
  return changes;
}

/** File contents for `rev:path` specs, or null where missing or binary, from one git process. */
export function readBlobs(specs) {
  if (specs.length === 0) return [];
  const out = git(["cat-file", "--batch"], specs.map((spec) => `${spec}\n`).join(""));
  const texts = [];
  let at = 0;
  for (let s = 0; s < specs.length; s++) {
    const eol = out.indexOf(10, at);
    const header = out.subarray(at, eol).toString("utf8");
    at = eol + 1;
    const match = /^\S+ (\S+) (\d+)$/.exec(header);
    if (!match) {
      texts.push(null);
      continue;
    }
    const size = Number(match[2]);
    const content = out.subarray(at, at + size);
    at += size + 1;
    texts.push(match[1] === "blob" && !content.subarray(0, 8000).includes(0) ? content.toString("utf8") : null);
  }
  return texts;
}

function main(argv) {
  const revs = { base: null, head: "HEAD" };
  for (let i = 0; i < argv.length; i++) {
    const name = /^--(base|head)$/.exec(argv[i])?.[1];
    if (name && argv[i + 1]) revs[name] = argv[++i];
    else {
      console.log(`::error::unknown argument ${argv[i]}; usage: node scripts/ci/check-test-integrity.mjs [--base <rev>] [--head <rev>]`);
      return 2;
    }
  }
  const { head } = revs;
  let { base } = revs;
  try {
    base ??= git(["merge-base", head, "origin/main"]).toString("utf8").trim();
  } catch {
    console.log("::error::no merge base with origin/main; fetch it, or pass --base <rev>");
    return 2;
  }
  const changes = changedFiles(base, head);
  const specs = changes.flatMap((c) => [
    c.status === "A" || c.status === "C" ? null : `${base}:${c.oldPath ?? c.path}`,
    c.status === "D" ? null : `${head}:${c.path}`,
  ]);
  const texts = readBlobs(specs.filter(Boolean));
  let next = 0;
  const contents = specs.map((spec) => (spec === null ? null : texts[next++]));
  const problems = checkChanges(
    changes.map((c, i) => ({ path: c.path, oldPath: c.oldPath, base: contents[2 * i], head: contents[2 * i + 1] })),
  );
  for (const { path, line, message } of problems) {
    console.log(`::error file=${path}${line ? `,line=${line}` : ""}::${message}`);
  }
  if (problems.length > 0) {
    console.log(
      `::error::This PR switches off ${problems.length === 1 ? "a check" : `${problems.length} checks`} that would judge it (AGENTS.md: 禁止事項 1, #630). ` +
        "If it is really needed, the owner says why in the PR and merges it.",
    );
    return 1;
  }
  console.log(`OK: none of the ${changes.length} changed files switches a check off.`);
  return 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exit(main(process.argv.slice(2)));
}
