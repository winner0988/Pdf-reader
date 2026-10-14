//! A ZIP archive written whole, in memory, with every file stored as it is (no compression): what a
//! .docx is made of (B3-02, docs/architecture/docx-export.md). Word and every other reader take
//! stored files like any other, and nothing here needs to be decompressed, so there is no
//! decompression bomb to defend against and no dependency to trust.
//!
//! The files get no dates and no author: the time of every one is 1 January 1980, the first the
//! format can say, so the archive tells nothing of when or where it was made.

/// What was asked of the archive does not fit it: more than it may hold, or more than ZIP can say
/// without its 64-bit extension, which is not written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

/// One file of the archive, as the central directory lists it.
struct Entry {
    name: Vec<u8>,
    crc: u32,
    size: u32,
    offset: u32,
}

/// A ZIP archive being written.
pub struct Archive {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
    limit: usize,
}

/// 1 January 1980 (the day the format counts from): year 0, month 1, day 1.
const DOS_DATE: u16 = (1 << 5) | 1;
/// Names and comments are UTF-8.
const FLAG_UTF8: u16 = 1 << 11;
/// Version 2.0: what stored files need.
const VERSION: u16 = 20;
const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_DIRECTORY: u32 = 0x0605_4b50;
/// Most files ZIP can list without its 64-bit extension.
const MAX_ENTRIES: usize = 0xFFFF;

impl Archive {
    /// An archive that may be `limit` bytes long at the most.
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            entries: Vec::new(),
            limit,
        }
    }

    /// Adds the file `name` with the contents `data`.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<(), Full> {
        let (size, offset) = (
            u32::try_from(data.len()).map_err(|_| Full)?,
            u32::try_from(self.bytes.len()).map_err(|_| Full)?,
        );
        let name_length = u16::try_from(name.len()).map_err(|_| Full)?;
        if self.entries.len() >= MAX_ENTRIES
            || self
                .bytes
                .len()
                .saturating_add(30 + name.len())
                .saturating_add(data.len())
                > self.limit
        {
            return Err(Full);
        }
        let crc = crc32(data);
        self.bytes.extend_from_slice(&LOCAL_HEADER.to_le_bytes());
        self.bytes.extend_from_slice(&VERSION.to_le_bytes());
        self.bytes.extend_from_slice(&FLAG_UTF8.to_le_bytes());
        // Stored, at midnight of the first day.
        self.bytes.extend_from_slice(&0u16.to_le_bytes());
        self.bytes.extend_from_slice(&0u16.to_le_bytes());
        self.bytes.extend_from_slice(&DOS_DATE.to_le_bytes());
        self.bytes.extend_from_slice(&crc.to_le_bytes());
        self.bytes.extend_from_slice(&size.to_le_bytes());
        self.bytes.extend_from_slice(&size.to_le_bytes());
        self.bytes.extend_from_slice(&name_length.to_le_bytes());
        self.bytes.extend_from_slice(&0u16.to_le_bytes());
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.extend_from_slice(data);
        self.entries.push(Entry {
            name: name.as_bytes().to_vec(),
            crc,
            size,
            offset,
        });
        Ok(())
    }

    /// The archive, with its directory.
    pub fn finish(mut self) -> Result<Vec<u8>, Full> {
        let directory = u32::try_from(self.bytes.len()).map_err(|_| Full)?;
        let count = u16::try_from(self.entries.len()).map_err(|_| Full)?;
        let listed: usize = self.entries.iter().map(|entry| 46 + entry.name.len()).sum();
        if self.bytes.len().saturating_add(listed).saturating_add(22) > self.limit {
            return Err(Full);
        }
        for entry in &self.entries {
            self.bytes.extend_from_slice(&CENTRAL_HEADER.to_le_bytes());
            // Made by (DOS, 2.0), needed (2.0).
            self.bytes.extend_from_slice(&VERSION.to_le_bytes());
            self.bytes.extend_from_slice(&VERSION.to_le_bytes());
            self.bytes.extend_from_slice(&FLAG_UTF8.to_le_bytes());
            self.bytes.extend_from_slice(&0u16.to_le_bytes());
            self.bytes.extend_from_slice(&0u16.to_le_bytes());
            self.bytes.extend_from_slice(&DOS_DATE.to_le_bytes());
            self.bytes.extend_from_slice(&entry.crc.to_le_bytes());
            self.bytes.extend_from_slice(&entry.size.to_le_bytes());
            self.bytes.extend_from_slice(&entry.size.to_le_bytes());
            let name_length = u16::try_from(entry.name.len()).map_err(|_| Full)?;
            self.bytes.extend_from_slice(&name_length.to_le_bytes());
            // No extra field, no comment, the first disk, no attributes.
            self.bytes.extend_from_slice(&[0; 2 + 2 + 2 + 2 + 4]);
            self.bytes.extend_from_slice(&entry.offset.to_le_bytes());
            self.bytes.extend_from_slice(&entry.name);
        }
        let size = u32::try_from(self.bytes.len() - directory as usize).map_err(|_| Full)?;
        self.bytes
            .extend_from_slice(&END_OF_DIRECTORY.to_le_bytes());
        // This disk and the one the directory starts on: the first.
        self.bytes.extend_from_slice(&[0; 4]);
        self.bytes.extend_from_slice(&count.to_le_bytes());
        self.bytes.extend_from_slice(&count.to_le_bytes());
        self.bytes.extend_from_slice(&size.to_le_bytes());
        self.bytes.extend_from_slice(&directory.to_le_bytes());
        self.bytes.extend_from_slice(&0u16.to_le_bytes());
        Ok(self.bytes)
    }
}

/// The table of CRC-32 (the one ZIP, PNG and Ethernet use), made when this is compiled.
const TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut value = n as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xEDB8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[n] = value;
        n += 1;
    }
    table
};

/// The CRC-32 of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut value = !0u32;
    for &byte in data {
        value = TABLE[((value ^ u32::from(byte)) & 0xFF) as usize] ^ (value >> 8);
    }
    !value
}

#[cfg(test)]
/// What the central directory of `archive` says of its files: name, contents.
pub(crate) fn read(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
    let le16 = |at: usize| usize::from(u16::from_le_bytes([archive[at], archive[at + 1]]));
    let le32 = |at: usize| {
        u32::from_le_bytes([
            archive[at],
            archive[at + 1],
            archive[at + 2],
            archive[at + 3],
        ])
    };
    let end = archive.len() - 22;
    assert_eq!(
        le32(end),
        END_OF_DIRECTORY,
        "the end of the directory is last"
    );
    let (count, size, start) = (
        le16(end + 10),
        le32(end + 12) as usize,
        le32(end + 16) as usize,
    );
    assert_eq!(start + size, end, "the directory is just before its end");
    let mut files = Vec::new();
    let mut at = start;
    for _ in 0..count {
        assert_eq!(le32(at), CENTRAL_HEADER);
        let (crc, stored, name_length) = (le32(at + 16), le32(at + 20) as usize, le16(at + 28));
        let local = le32(at + 42) as usize;
        let name = String::from_utf8(archive[at + 46..at + 46 + name_length].to_vec()).unwrap();
        // The local header says the same, and the data follows its name.
        assert_eq!(le32(local), LOCAL_HEADER);
        assert_eq!(le32(local + 14), crc);
        assert_eq!(le32(local + 18) as usize, stored);
        assert_eq!(le16(local + 26), name_length);
        let data_at = local + 30 + name_length + le16(local + 28);
        let data = archive[data_at..data_at + stored].to_vec();
        assert_eq!(crc32(&data), crc, "{name}");
        files.push((name, data));
        at += 46 + name_length;
    }
    assert_eq!(at, end, "every file is listed");
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_is_the_one_of_zip() {
        // The check value of the standard (CRC-32/ISO-HDLC).
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
    }

    #[test]
    fn what_is_added_is_listed_and_comes_back_as_it_was() {
        let mut archive = Archive::new(1 << 20);
        archive.add("[Content_Types].xml", b"<Types/>").unwrap();
        archive
            .add("word/media/image1.png", &[0, 1, 2, 255, 0])
            .unwrap();
        archive.add("word/空.xml", b"").unwrap();
        let bytes = archive.finish().unwrap();
        let files = read(&bytes);
        assert_eq!(
            files,
            vec![
                ("[Content_Types].xml".to_owned(), b"<Types/>".to_vec()),
                ("word/media/image1.png".to_owned(), vec![0, 1, 2, 255, 0]),
                ("word/空.xml".to_owned(), Vec::new()),
            ]
        );
        // It starts as a ZIP file does.
        assert_eq!(&bytes[..4], b"PK\x03\x04");
    }

    #[test]
    fn nothing_of_when_or_by_whom_is_in_it() {
        let mut archive = Archive::new(1 << 20);
        archive.add("a.txt", b"x").unwrap();
        let bytes = archive.finish().unwrap();
        // The time of the local header is 0:00 and its date 1 January 1980.
        assert_eq!(&bytes[10..14], &[0, 0, 0x21, 0]);
        // And the same in the directory.
        let directory = bytes.len() - 22 - (46 + 5);
        assert_eq!(&bytes[directory + 12..directory + 16], &[0, 0, 0x21, 0]);
    }

    #[test]
    fn it_stops_at_the_size_it_may_have() {
        let mut archive = Archive::new(100);
        assert!(archive.add("a", &[0; 10]).is_ok());
        assert_eq!(archive.add("b", &[0; 100]), Err(Full));
        // What is there fits, but not with its directory.
        let mut tight = Archive::new(30 + 1 + 10 + 22);
        tight.add("a", &[0; 10]).unwrap();
        assert_eq!(tight.finish(), Err(Full));
        let mut fits = Archive::new(30 + 1 + 10 + 46 + 1 + 22);
        fits.add("a", &[0; 10]).unwrap();
        assert!(fits.finish().is_ok());
    }

    #[test]
    fn it_lists_no_more_files_than_zip_can_count() {
        let mut archive = Archive::new(usize::MAX);
        for number in 0..MAX_ENTRIES {
            archive.add(&number.to_string(), b"").unwrap();
        }
        assert_eq!(archive.add("one more", b""), Err(Full));
        assert!(archive.finish().is_ok());
    }
}
