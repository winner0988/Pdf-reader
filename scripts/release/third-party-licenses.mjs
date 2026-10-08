#!/usr/bin/env node
// The notices of what the installer ships (ADR 0011, REL-04): every Rust crate and JavaScript package
// that ends up in the two programs, MuPDF and the libraries built into it, the OCR language
// data and the fonts, each with its licence, and each licence text once.
//
//   node scripts/release/third-party-licenses.mjs           writes src/assets/THIRD_PARTY_LICENSES.txt
//   node scripts/release/third-party-licenses.mjs --check   fails if the file is not what this makes
//
// Nothing here is fetched: the texts are the files the dependencies came with (cargo's registry, the
// node_modules of pnpm). A package that came without its licence text is given the standard text
// of its licence, with the authors its manifest names, only for the licences below; any other
// stops the script, so that a new dependency is looked at by a person (docs/release.md).
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const OUTPUT = path.join(ROOT, "src", "assets", "THIRD_PARTY_LICENSES.txt");
const REPOSITORY = "https://github.com/winner0988/Pdf-reader";
const TARGET = "x86_64-pc-windows-msvc";
/** The two programs the installer puts on the disk (the main program and the worker). */
const SHIPPED = ["pdf-reader", "pdf_worker"];
/** The files a package keeps its licence in. */
const LICENCE_FILE = /^(licen[sc]e|copying|notice|unlicense|copyright)([-_. ].*)?$/i;
const MAX_TEXT_BYTES = 200 * 1024;

/** A text as it is compared and written: LF line ends, no trailing space, no BOM. */
export function normalise(text) {
  return text
    .replace(/^﻿/, "")
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.replace(/[ \t]+$/, ""))
    .join("\n")
    .trim();
}

/** Two texts are the same licence text when they only differ in white space. */
export function textKey(text) {
  return createHash("sha256").update(text.replace(/\s+/g, " ")).digest("hex");
}

function read(file) {
  const text = normalise(readFileSync(file, "utf8"));
  if (Buffer.byteLength(text) > MAX_TEXT_BYTES) throw new Error(`${file} is larger than ${MAX_TEXT_BYTES} bytes`);
  return text;
}

/** The licence files that sit directly in `dir`. */
function licenceFiles(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isFile() && LICENCE_FILE.test(entry.name))
    .map((entry) => path.join(dir, entry.name))
    .sort();
}

/** The standard texts of the licences a package may have come without. */
export function standardText(spdx, holders) {
  const who = holders.length ? holders.join(", ") : "the authors of the package";
  if (/\bMIT\b/.test(spdx)) {
    return normalise(`MIT License

Copyright (c) ${who}

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT
OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.`);
  }
  if (spdx === "BSD-3-Clause") {
    return normalise(`BSD 3-Clause License

Copyright (c) ${who}. All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted
provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this list of conditions
   and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice, this list of
   conditions and the following disclaimer in the documentation and/or other materials provided
   with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors may be used to
   endorse or promote products derived from this software without specific prior written
   permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR
IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND
FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR
CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER
IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF
THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`);
  }
  return null;
}

/** What `cargo metadata` knows of the crates that end up in the shipped programs (not this project's own). */
export function rustPackages() {
  const meta = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET], {
      cwd: ROOT,
      maxBuffer: 1 << 29,
      encoding: "utf8",
    }),
  );
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((node) => [node.id, node]));
  const roots = meta.packages.filter((p) => SHIPPED.includes(p.name)).map((p) => p.id);
  if (roots.length !== SHIPPED.length) throw new Error("the shipped programs are not in `cargo metadata`");
  const reached = new Set();
  const pending = [...roots];
  while (pending.length) {
    const id = pending.pop();
    if (reached.has(id)) continue;
    reached.add(id);
    // Normal dependencies only (`kind: null`): what is compiled in, not the tools that built it.
    for (const dep of nodes.get(id).deps) if (dep.dep_kinds.some((kind) => kind.kind === null)) pending.push(dep.pkg);
  }
  return [...reached]
    .map((id) => byId.get(id))
    .filter((p) => p.source !== null)
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** The packages of the front end that are not only used to build it. */
export function npmPackages() {
  const listed = JSON.parse(
    execFileSync("pnpm", ["licenses", "list", "--prod", "--json"], { cwd: ROOT, maxBuffer: 1 << 28, encoding: "utf8", shell: true }),
  );
  const packages = [];
  for (const [license, list] of Object.entries(listed)) {
    for (const item of list) {
      for (const [index, version] of item.versions.entries()) {
        packages.push({ name: item.name, version, license, url: item.homepage ?? "", dir: item.paths[index], holders: [item.author].filter(Boolean) });
      }
    }
  }
  return packages.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** A comment block of a source file, as plain text (Leptonica keeps its licence there). */
export function commentBlock(source) {
  const block = source.match(/^\/\*=+\*\n([\s\S]*?)\*=+\*\//);
  if (!block) return null;
  return normalise(block[1].replace(/^ -$/gm, "").replace(/^ -  ?/gm, ""));
}

/** The libraries that MuPDF's sources carry, and the files that say how they may be used. */
export function mupdfLibraries(dir) {
  const third = (name) => path.join(dir, "thirdparty", name);
  const files = (name, ...names) => names.map((file) => read(path.join(third(name), file)));
  // The libraries MuPDF's Windows build compiles in (not the tools: curl, freeglut, zint, zxing,
  // the OpenSSL signing and the JavaScript engine, which is left out).
  return [
    { name: "Brotli", license: "MIT", url: "https://github.com/google/brotli", texts: files("brotli", "LICENSE") },
    { name: "extract", license: "see the text", url: "https://ghostscript.com/", texts: files("extract", "COPYING") },
    { name: "FreeType", license: "FreeType License or GPL-2.0-or-later", url: "https://freetype.org/", texts: files("freetype", "LICENSE.TXT") },
    { name: "Gumbo", license: "Apache-2.0", url: "https://github.com/google/gumbo-parser", texts: files("gumbo-parser", "COPYING") },
    { name: "HarfBuzz", license: "MIT", url: "https://harfbuzz.github.io/", texts: files("harfbuzz", "COPYING") },
    { name: "jbig2dec", license: "AGPL-3.0-or-later", url: "https://jbig2dec.com/", texts: files("jbig2dec", "LICENSE") },
    { name: "Little CMS", license: "MIT", url: "https://www.littlecms.com/", texts: files("lcms2", "LICENSE") },
    {
      name: "Leptonica",
      license: "BSD-2-Clause",
      url: "https://github.com/DanBloomberg/leptonica",
      texts: [commentBlock(readFileSync(path.join(third("leptonica"), "src", "allheaders.h"), "utf8"))],
    },
    {
      name: "libjpeg (Independent JPEG Group)",
      license: "IJG",
      url: "https://www.ijg.org/",
      texts: ["This software is based in part on the work of the Independent JPEG Group."],
    },
    { name: "OpenJPEG", license: "BSD-2-Clause", url: "https://www.openjpeg.org/", texts: files("openjpeg", "LICENSE") },
    { name: "Tesseract OCR", license: "Apache-2.0", url: "https://github.com/tesseract-ocr/tesseract", texts: files("tesseract", "LICENSE") },
    { name: "zlib", license: "Zlib", url: "https://zlib.net/", texts: files("zlib", "LICENSE") },
  ];
}

const authorsOf = (p) => (p.authors ?? []).map((author) => author.replace(/\s*<[^>]*>/, "").trim()).filter(Boolean);

/** What goes into the file, before the texts are given their numbers. */
export function collect() {
  const rust = rustPackages();
  const sys = rust.find((p) => p.name === "mupdf-sys");
  if (!sys) throw new Error("mupdf-sys is not among the dependencies");
  const mupdf = path.join(path.dirname(sys.manifest_path), "mupdf");
  const mupdfVersion = readFileSync(path.join(mupdf, "include", "mupdf", "fitz", "version.h"), "utf8").match(/FZ_VERSION "([^"]+)"/)?.[1];
  if (!mupdfVersion) throw new Error("no MuPDF version in version.h");
  const mupdfLicence = read(path.join(mupdf, "COPYING"));
  const libraries = mupdfLibraries(mupdf);
  const tesseract = libraries.find((library) => library.name === "Tesseract OCR").texts[0];

  const sections = [];
  sections.push({
    title: "PDF Reader",
    entries: [
      { name: "PDF Reader", license: "AGPL-3.0-or-later", url: REPOSITORY, texts: [read(path.join(ROOT, "LICENSE"))] },
    ],
  });
  sections.push({
    title: "MuPDF, and the libraries built into it",
    entries: [
      {
        name: "MuPDF",
        version: mupdfVersion,
        license: "AGPL-3.0",
        url: "https://mupdf.com/",
        note: `The sources are those of the Rust crate mupdf-sys ${sys.version}, which carries them.`,
        texts: [mupdfLicence],
      },
      ...libraries.map((library) => ({ ...library, note: "Built into MuPDF." })),
      {
        name: "URW++ base 35 fonts",
        license: "distributed with MuPDF, under its licence",
        url: "https://mupdf.com/",
        note: "The standard PDF fonts, built into MuPDF.",
        texts: [mupdfLicence],
      },
    ],
  });
  sections.push({
    title: "Data",
    entries: [
      {
        name: "Tesseract language data (tessdata_fast: eng, chi_tra)",
        license: "Apache-2.0",
        url: "https://github.com/tesseract-ocr/tessdata_fast",
        note: "Installed next to the program, unchanged (src-tauri/resources/tessdata/README.md).",
        texts: [tesseract],
      },
    ],
  });

  // The texts the packages came with decide what a package without one is given.
  const common = new Map();
  const found = rust.map((p) => {
    const dir = path.dirname(p.manifest_path);
    const texts = [...new Map(licenceFiles(dir).map((file) => read(file)).map((text) => [textKey(text), text])).values()];
    if (p.license && texts.length === 1) {
      const counts = common.get(p.license) ?? new Map();
      counts.set(textKey(texts[0]), { text: texts[0], count: (counts.get(textKey(texts[0]))?.count ?? 0) + 1 });
      common.set(p.license, counts);
    }
    return { p, texts };
  });
  const commonText = (license) => [...(common.get(license)?.values() ?? [])].sort((a, b) => b.count - a.count)[0]?.text;
  const crates = [];
  for (const { p, texts } of found) {
    let note;
    if (p.name === "mupdf-sys") {
      texts.push(mupdfLicence);
      note = "Bindings and build of MuPDF (above).";
    } else if (texts.length === 0) {
      const holders = authorsOf(p).length ? authorsOf(p) : [`the contributors of ${p.repository ?? p.name}`];
      const standard = standardText(p.license ?? "", holders) ?? commonText(p.license);
      if (!standard) throw new Error(`${p.name} ${p.version} (${p.license}) came without a licence text, and this script has no standard text for it: add one`);
      texts.push(standard);
      note = "The package has no licence file: this is the standard text of its licence.";
    }
    crates.push({ name: p.name, version: p.version, license: p.license ?? "see the text", url: p.repository ?? p.homepage ?? "", texts, note });
  }
  sections.push({ title: "Rust crates", entries: crates });

  const packages = npmPackages().map((p) => {
    const texts = licenceFiles(p.dir).map((file) => read(file));
    if (texts.length === 0) throw new Error(`${p.name} ${p.version} has no licence file`);
    return { name: p.name, version: p.version, license: p.license, url: p.url, texts };
  });
  sections.push({ title: "JavaScript packages", entries: packages });
  return sections;
}

const HEADER = `PDF Reader: third-party notices
第三方元件授權聲明

PDF Reader is free software under the GNU Affero General Public License, version 3 or (at your
option) any later version. The complete corresponding source code of each release is at
${REPOSITORY} (the tag of the version, v<version>).
PDF Reader 以 AGPL-3.0-or-later 授權；每個版本的完整原始碼在上面的網址（該版本的 tag）。

PDF Reader contains the components below, each under its own licence. The licence texts are the
originals; only they have legal effect. A text is written once and referred to by its number
(L1, L2, ...). Components under the MPL-2.0 are available as source from the address of the
component (for Rust crates also on https://crates.io/crates/<name>).
以下元件各自保留自己的授權；授權全文只有原文有法律效力，每一份全文只列一次，以編號（L1、L2…）對照。`;

/** The file: the components of each section, then every licence text once. */
export function render(sections) {
  const texts = new Map();
  const label = (entry) => (entry.version ? `${entry.name} ${entry.version}` : entry.name);
  for (const section of sections) {
    for (const entry of section.entries) {
      entry.ids = entry.texts.map((text) => {
        const key = textKey(text);
        if (!texts.has(key)) texts.set(key, { id: texts.size + 1, text, users: [] });
        const known = texts.get(key);
        known.users.push(label(entry));
        return `L${known.id}`;
      });
    }
  }
  const lines = [HEADER, ""];
  for (const section of sections) {
    lines.push(`${"=".repeat(78)}`, section.title, `${"=".repeat(78)}`);
    for (const entry of section.entries) {
      const detail = [entry.license, entry.url].filter(Boolean).join(" | ");
      lines.push(`- ${label(entry)} | ${detail} | ${entry.ids.join(", ")}`);
      if (entry.note) lines.push(`    ${entry.note}`);
    }
    lines.push("");
  }
  lines.push("=".repeat(78), "Licence texts", "=".repeat(78), "");
  for (const { id, text, users } of texts.values()) {
    lines.push(`----- L${id} -----`, `Used by: ${[...new Set(users)].join(", ")}`, "", text, "");
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

export function generate() {
  return render(collect());
}

if (fileURLToPath(import.meta.url) === path.resolve(process.argv[1] ?? "")) {
  const check = process.argv.includes("--check");
  const text = generate();
  if (check) {
    const current = existsSync(OUTPUT) ? readFileSync(OUTPUT, "utf8").replace(/\r\n/g, "\n") : "";
    if (current !== text) {
      console.error(
        `${path.relative(ROOT, OUTPUT)} is not what the dependencies give: run node scripts/release/third-party-licenses.mjs and commit it.`,
      );
      process.exit(1);
    }
    console.log(`${path.relative(ROOT, OUTPUT)} is up to date.`);
  } else {
    writeFileSync(OUTPUT, text);
    console.log(`wrote ${path.relative(ROOT, OUTPUT)} (${Buffer.byteLength(text)} bytes)`);
  }
}
