import assert from "node:assert/strict";
import { test } from "node:test";

import { findExternalReferences } from "./check-dist.mjs";

test("local references in HTML are accepted", () => {
  const html = `<script type="module" crossorigin src="/assets/index-abc.js"></script>
    <link rel="stylesheet" href="/assets/index-abc.css"><link rel="icon" href="data:,">`;
  assert.deepEqual(findExternalReferences(html, "html"), []);
});

test("external references in HTML are found", () => {
  for (const html of [
    `<script src="https://cdn.jsdelivr.net/npm/x"></script>`,
    `<link href='//fonts.googleapis.com/css2?family=Inter' rel="stylesheet">`,
    `<img src=http://tracker.example.invalid/pixel.gif>`,
    `<form action="https://example.invalid/submit"></form>`,
  ]) {
    assert.equal(findExternalReferences(html, "html").length, 1, html);
  }
});

test("data attributes are not mistaken for the data= attribute", () => {
  assert.deepEqual(findExternalReferences(`<div data-url="https://example.invalid"></div>`, "html"), []);
});

test("external url() and @import in CSS are found", () => {
  for (const css of [
    `@import url("https://fonts.googleapis.com/css2?family=Inter");`,
    `@import "https://example.invalid/theme.css";`,
    `.a { background: url(//example.invalid/bg.png) }`,
    `@font-face { src: url('https://fonts.gstatic.com/x.woff2') }`,
  ]) {
    assert.equal(findExternalReferences(css, "css").length, 1, css);
  }
});

test("licence comments and local urls in CSS are accepted", () => {
  const css = `/*! tailwindcss v4 | MIT License | https://tailwindcss.com */ .a { background: url(/assets/bg.png) } .b { mask: url(data:image/svg+xml;base64,AAA=) }`;
  assert.deepEqual(findExternalReferences(css, "css"), []);
});
