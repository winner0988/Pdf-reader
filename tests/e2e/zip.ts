// Reading a ZIP archive whose files are stored as they are, which is how the Word export writes
// one (B3-02): enough to look into a .docx in a test, with no dependency.
import { crc32 } from "node:zlib";

/** The files of `zip` by name; each one's CRC-32 is checked. Throws for anything else a ZIP can be. */
export function storedFiles(zip: Buffer): Record<string, Buffer> {
  const end = zip.length - 22;
  if (end < 0 || zip.readUInt32LE(end) !== 0x06054b50) throw new Error("the archive does not end with its directory");
  const count = zip.readUInt16LE(end + 10);
  let at = zip.readUInt32LE(end + 16);
  const files: Record<string, Buffer> = {};
  for (let n = 0; n < count; n++) {
    if (zip.readUInt32LE(at) !== 0x02014b50) throw new Error(`entry ${n} is not a directory entry`);
    const method = zip.readUInt16LE(at + 10);
    const crc = zip.readUInt32LE(at + 16);
    const size = zip.readUInt32LE(at + 20);
    const nameLength = zip.readUInt16LE(at + 28);
    const extraLength = zip.readUInt16LE(at + 30);
    const commentLength = zip.readUInt16LE(at + 32);
    const local = zip.readUInt32LE(at + 42);
    const name = zip.subarray(at + 46, at + 46 + nameLength).toString("utf8");
    if (method !== 0) throw new Error(`${name} is compressed (method ${method})`);
    if (zip.readUInt32LE(local) !== 0x04034b50) throw new Error(`${name} has no local header`);
    const data = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28);
    files[name] = zip.subarray(data, data + size);
    if (crc32(files[name]!) !== crc) throw new Error(`${name} does not match its CRC-32`);
    at += 46 + nameLength + extraLength + commentLength;
  }
  return files;
}
