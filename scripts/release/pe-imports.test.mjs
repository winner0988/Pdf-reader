import assert from "node:assert/strict";
import { test } from "node:test";

import { importedDlls, redistributableDlls } from "./pe-imports.mjs";

/** A minimal PE32+ image with one .idata section holding import and delay-import tables. */
function image(dlls, delayDlls = []) {
  const bytes = Buffer.alloc(0x400);
  const sectionRva = 0x1000;
  const sectionRaw = 0x200;
  const raw = (rva) => rva - sectionRva + sectionRaw;

  bytes.writeUInt16LE(0x5a4d, 0); // MZ
  bytes.writeUInt32LE(0x40, 0x3c);
  bytes.writeUInt32LE(0x00004550, 0x40); // PE\0\0
  const coff = 0x44;
  bytes.writeUInt16LE(0x8664, coff); // x64
  bytes.writeUInt16LE(1, coff + 2); // one section
  bytes.writeUInt16LE(240, coff + 16); // PE32+ optional header size
  const optional = coff + 20;
  bytes.writeUInt16LE(0x20b, optional); // PE32+
  bytes.writeUInt32LE(16, optional + 108); // NumberOfRvaAndSizes
  const directories = optional + 112;

  const section = optional + 240;
  bytes.write(".idata", section, "latin1");
  bytes.writeUInt32LE(0x200, section + 8);
  bytes.writeUInt32LE(sectionRva, section + 12);
  bytes.writeUInt32LE(0x200, section + 16);
  bytes.writeUInt32LE(sectionRaw, section + 20);

  let name = 0x1100;
  const writeName = (text) => {
    bytes.write(`${text}\0`, raw(name), "latin1");
    const rva = name;
    name += text.length + 1;
    return rva;
  };
  dlls.forEach((dll, index) => {
    const entry = raw(0x1000 + index * 20);
    bytes.writeUInt32LE(writeName(dll), entry + 12);
    bytes.writeUInt32LE(0x1300, entry + 16); // FirstThunk (not read)
  });
  bytes.writeUInt32LE(0x1000, directories + 1 * 8);
  bytes.writeUInt32LE((dlls.length + 1) * 20, directories + 1 * 8 + 4);
  if (delayDlls.length > 0) {
    delayDlls.forEach((dll, index) => {
      bytes.writeUInt32LE(writeName(dll), raw(0x1080 + index * 32) + 4);
    });
    bytes.writeUInt32LE(0x1080, directories + 13 * 8);
    bytes.writeUInt32LE((delayDlls.length + 1) * 32, directories + 13 * 8 + 4);
  }
  return bytes;
}

test("lists imports and delay imports", () => {
  const result = importedDlls(image(["KERNEL32.dll", "ntdll.dll"], ["bcrypt.dll"]));
  assert.deepEqual(result, {
    imports: ["KERNEL32.dll", "ntdll.dll"],
    delayImports: ["bcrypt.dll"],
  });
});

test("a statically linked image needs no redistributable", () => {
  const bytes = image(["KERNEL32.dll", "api-ms-win-core-synch-l1-2-0.dll", "ntdll.dll"]);
  assert.deepEqual(redistributableDlls(bytes), []);
});

test("flags the VC++ runtime, C++ library and debug CRT, also when delay-loaded", () => {
  const bytes = image(
    ["KERNEL32.dll", "MSVCP140.dll", "VCRUNTIME140.dll", "VCRUNTIME140_1.dll"],
    ["ucrtbased.dll", "MSVCP140_ATOMIC_WAIT.dll"],
  );
  assert.deepEqual(redistributableDlls(bytes), [
    "MSVCP140.dll",
    "VCRUNTIME140.dll",
    "VCRUNTIME140_1.dll",
    "ucrtbased.dll",
    "MSVCP140_ATOMIC_WAIT.dll",
  ]);
});

test("the Windows UCRT is not a redistributable", () => {
  assert.deepEqual(redistributableDlls(image(["ucrtbase.dll", "api-ms-win-crt-heap-l1-1-0.dll"])), []);
});

test("rejects files that are not PE images", () => {
  assert.throws(() => importedDlls(Buffer.from("%PDF-1.7\n")), /not a PE image/);
  const noSignature = image([]);
  noSignature.writeUInt32LE(0, 0x40);
  assert.throws(() => importedDlls(noSignature), /no PE signature/);
});
