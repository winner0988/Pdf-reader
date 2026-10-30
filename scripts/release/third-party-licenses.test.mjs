import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { commentBlock, compiledPackages, normalise, render, standardText, textKey } from "./third-party-licenses.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

test("a text is compared without its white space, its BOM and its line ends", () => {
  assert.equal(normalise("﻿  MIT License  \r\nCopyright (c) x \r\n\r\n"), "MIT License\nCopyright (c) x");
  assert.equal(textKey("a  b\r\nc"), textKey("a b c"));
  assert.notEqual(textKey("Copyright (c) 2020 A"), textKey("Copyright (c) 2021 A"));
});

test("Leptonica's licence is the comment at the top of one of its headers", () => {
  const header = [
    "/*====================================================================*",
    " -  Copyright (C) 2001 Leptonica.  All rights reserved.",
    " -",
    " -  Redistribution and use in source and binary forms are permitted.",
    " *====================================================================*/",
    "",
    "#ifndef X",
  ].join("\n");
  assert.equal(
    commentBlock(header),
    "Copyright (C) 2001 Leptonica.  All rights reserved.\n\nRedistribution and use in source and binary forms are permitted.",
  );
  assert.equal(commentBlock("int main() {}"), null);
});

test("a package that came without its licence text gets the standard text of MIT or BSD-3-Clause, and no other", () => {
  assert.match(standardText("MIT", ["Jane Public"]), /^MIT License\n\nCopyright \(c\) Jane Public\n/);
  assert.match(standardText("MIT OR Apache-2.0", ["A", "B"]), /Copyright \(c\) A, B\n/);
  assert.match(standardText("BSD-3-Clause", ["Jane Public"]), /^BSD 3-Clause License\n\nCopyright \(c\) Jane Public\./);
  for (const other of ["MPL-2.0", "GPL-3.0", "Apache-2.0", "Unlicense", ""]) assert.equal(standardText(other, []), null, other);
});

test("a text many components share is written once, numbered in the order it is met", () => {
  const sections = [
    {
      title: "One",
      entries: [
        { name: "alpha", version: "1.0.0", license: "MIT", url: "https://a.invalid", texts: ["MIT text A"] },
        { name: "beta", version: "2.0.0", license: "MIT", url: "", texts: ["MIT  text   A"] },
        { name: "gamma", license: "Apache-2.0", url: "", texts: ["Apache text", "MIT text A"], note: "A note." },
      ],
    },
  ];
  const text = render(sections);
  assert.match(text, /^- alpha 1\.0\.0 \| MIT \| https:\/\/a\.invalid \| L1$/m);
  assert.match(text, /^- beta 2\.0\.0 \| MIT \| L1$/m);
  assert.match(text, /^- gamma \| Apache-2\.0 \| L2, L1\n {4}A note\.$/m);
  assert.equal([...text.matchAll(/^----- L\d+ -----$/gm)].length, 2);
  assert.match(text, /----- L1 -----\nUsed by: alpha 1\.0\.0, beta 2\.0\.0, gamma\n\nMIT text A\n/);
  assert.ok(text.endsWith("\n") && !text.endsWith("\n\n"));
});

test("an optional dependency that only a weak feature names is not compiled in, one a feature turns on is (#397)", () => {
  // app needs lib with its feature `std`, which is `weak?/std`: that turns on `std` of `weak` only
  // if something else turns `weak` on, and nothing does. Lib's default feature turns `named` on.
  const manifest = (name, rest) => `[package]\nname = "${name}"\nversion = "0.1.0"\nedition = "2021"\n\n${rest}\n`;
  const files = {
    "Cargo.toml": '[workspace]\nmembers = ["app"]\nresolver = "2"\n',
    "app/Cargo.toml": manifest("app", '[dependencies]\nlib = { path = "../lib", features = ["std"] }'),
    "app/src/main.rs": "fn main() {}\n",
    "lib/Cargo.toml": manifest(
      "lib",
      '[dependencies]\nnamed = { path = "../named", optional = true }\nweak = { path = "../weak", optional = true }\n\n' +
        '[features]\ndefault = ["named"]\nstd = ["weak?/std"]',
    ),
    "lib/src/lib.rs": "",
    "named/Cargo.toml": manifest("named", ""),
    "named/src/lib.rs": "",
    "weak/Cargo.toml": manifest("weak", "[features]\nstd = []"),
    "weak/src/lib.rs": "",
  };
  const dir = mkdtempSync(path.join(tmpdir(), "notices-"));
  try {
    for (const [name, text] of Object.entries(files)) {
      mkdirSync(path.dirname(path.join(dir, name)), { recursive: true });
      writeFileSync(path.join(dir, name), text);
    }
    // The repository's toolchain, the one that is installed.
    copyFileSync(path.join(ROOT, "rust-toolchain.toml"), path.join(dir, "rust-toolchain.toml"));
    execFileSync("cargo", ["generate-lockfile", "--offline"], { cwd: dir, stdio: "ignore" });

    const names = compiledPackages(dir, ["app"]).map((p) => p.name).sort();
    assert.deepEqual(names, ["app", "lib", "named"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
