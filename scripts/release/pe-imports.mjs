// Reads the DLL imports of a Windows PE image (REL-01). Used to prove pdf_worker.exe does not
// need the Visual C++ redistributable, which a clean Windows 11 does not have.

/** DLLs that come from the VC++ redistributable (or the debug CRT), not from Windows itself. */
const REDISTRIBUTABLE = /^(msvcp\d+.*|vcruntime\d+.*|msvcr\d+.*|vcomp\d+.*|concrt\d+.*|vccorlib\d+.*|ucrtbased)\.dll$/i;

/**
 * Returns the DLL names in the import and delay-import directories of a PE32 or PE32+ image.
 * @param {Uint8Array} bytes
 * @returns {{ imports: string[], delayImports: string[] }}
 */
export function importedDlls(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const u16 = (offset) => view.getUint16(offset, true);
  const u32 = (offset) => view.getUint32(offset, true);

  if (bytes.byteLength < 0x40 || u16(0) !== 0x5a4d) throw new Error("not a PE image (no MZ header)");
  const pe = u32(0x3c);
  if (u32(pe) !== 0x00004550) throw new Error("not a PE image (no PE signature)");
  const coff = pe + 4;
  const sectionCount = u16(coff + 2);
  const optionalSize = u16(coff + 16);
  const optional = coff + 20;
  const magic = u16(optional);
  const directories = optional + (magic === 0x20b ? 112 : magic === 0x10b ? 96 : NaN);
  if (Number.isNaN(directories)) throw new Error(`unknown optional header magic 0x${magic.toString(16)}`);
  const directoryCount = u32(directories - 4);
  const sections = optional + optionalSize;

  const offsetOf = (rva) => {
    for (let index = 0; index < sectionCount; index++) {
      const header = sections + index * 40;
      const virtualSize = u32(header + 8);
      const virtualAddress = u32(header + 12);
      const rawSize = u32(header + 16);
      const rawOffset = u32(header + 20);
      if (rva >= virtualAddress && rva < virtualAddress + Math.max(virtualSize, rawSize)) {
        return rva - virtualAddress + rawOffset;
      }
    }
    throw new Error(`RVA 0x${rva.toString(16)} is outside every section`);
  };
  const nameAt = (rva) => {
    const start = offsetOf(rva);
    let end = start;
    while (end < bytes.byteLength && bytes[end] !== 0) end++;
    return new TextDecoder("latin1").decode(bytes.subarray(start, end));
  };
  const directory = (index) => (index < directoryCount ? u32(directories + index * 8) : 0);

  const imports = [];
  const importRva = directory(1);
  if (importRva) {
    // IMAGE_IMPORT_DESCRIPTOR: 20 bytes, Name RVA at +12, FirstThunk at +16; ends with zeros.
    for (let entry = offsetOf(importRva); ; entry += 20) {
      const name = u32(entry + 12);
      if (name === 0 && u32(entry + 16) === 0) break;
      imports.push(nameAt(name));
    }
  }

  const delayImports = [];
  const delayRva = directory(13);
  if (delayRva) {
    // IMAGE_DELAYLOAD_DESCRIPTOR: 32 bytes, DllNameRVA at +4; ends with zeros.
    for (let entry = offsetOf(delayRva); ; entry += 32) {
      const name = u32(entry + 4);
      if (name === 0) break;
      delayImports.push(nameAt(name));
    }
  }
  return { imports, delayImports };
}

/** Imported DLLs that would require the VC++ redistributable. Empty means none. */
export function redistributableDlls(bytes) {
  const { imports, delayImports } = importedDlls(bytes);
  return [...imports, ...delayImports].filter((name) => REDISTRIBUTABLE.test(name));
}
