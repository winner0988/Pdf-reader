//! A PDF made of files, through the real worker in the real sandbox (B3-03): the worker reads each
//! source through a read-only handle (never a path) and writes the PDF to a write-only one, what
//! it makes is the words of the source and nothing about it, and a hostile source ends in an
//! error, never in anything but the worker's own end.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ipc_contract::limits::{MAX_CREATE_SOURCES, MAX_SOURCE_BYTES};
use ipc_contract::types::{CreateKind, ErrorCode};
use ipc_contract::worker::{WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, HostError, WorkerHost};

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// A path in the temp directory no other test uses.
fn temp_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "pdf-worker-{}-create-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A PDF made, and where it is.
struct Made {
    path: PathBuf,
    pages: u32,
    bytes: Vec<u8>,
}

impl Drop for Made {
    fn drop(&mut self) {
        std::fs::remove_file(&self.path).ok();
    }
}

/// What the worker makes of the files `sources`.
fn create(host: &mut WorkerHost, sources: &[(&Path, CreateKind)]) -> Result<Made, HostError> {
    let files: Vec<File> = sources
        .iter()
        .map(|(path, _)| File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .collect();
    let handed: Vec<(&File, CreateKind)> = files
        .iter()
        .zip(sources)
        .map(|(file, (_, kind))| (file, *kind))
        .collect();
    let path = temp_path("made.pdf");
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create the output");
    let result = host.create_pdf(&handed, &output);
    drop(output);
    let made = |pages, bytes: u64| {
        let made = Made {
            path: path.clone(),
            pages,
            bytes: std::fs::read(&path).expect("the PDF"),
        };
        assert_eq!(made.bytes.len() as u64, bytes);
        made
    };
    match result {
        Ok(WorkerResponse::Created { bytes, pages, .. }) => Ok(made(pages, bytes)),
        Ok(other) => panic!("{other:?}"),
        Err(error) => {
            std::fs::remove_file(&path).ok();
            Err(error)
        }
    }
}

/// The words on all pages of `made`, as the worker reads them.
fn words(host: &mut WorkerHost, made: &Made) -> String {
    let (doc, opened) = host.open(&made.path).expect("open the PDF");
    let WorkerResponse::Opened { document, .. } = opened else {
        panic!("{opened:?}");
    };
    assert_eq!(document.pages.len() as u32, made.pages);
    let mut text = String::new();
    for page_index in 0..made.pages {
        let response = host
            .request(|request| WorkerRequest::GetPageText {
                request,
                doc,
                page_index,
            })
            .expect("page text");
        let WorkerResponse::PageText { text: page, .. } = response else {
            panic!("{response:?}");
        };
        for line in page.lines {
            text.push_str(&line.text);
            text.push('\n');
        }
    }
    text
}

fn code(error: &HostError) -> ErrorCode {
    error.code()
}

#[test]
fn every_kind_of_file_becomes_a_pdf_of_its_words_and_nothing_of_its_own() {
    let cases: [(&str, CreateKind, &[&str]); 8] = [
        (
            "sources/text.txt",
            CreateKind::Text,
            &["Hello text file", "純文字檔"],
        ),
        (
            "sources/web-page.html",
            CreateKind::Html,
            &["Web page heading", "你好"],
        ),
        ("sources/picture.svg", CreateKind::Svg, &["Hello SVG"]),
        (
            "sources/book.epub",
            CreateKind::Epub,
            &["Chapter one", "first words"],
        ),
        ("sources/page.xps", CreateKind::Xps, &[]),
        (
            "sources/letter.docx",
            CreateKind::Docx,
            &["First paragraph of the letter", "隱私"],
        ),
        (
            "sources/sheet.xlsx",
            CreateKind::Xlsx,
            &["Alpha cell", "4242"],
        ),
        (
            "sources/slides.pptx",
            CreateKind::Pptx,
            &["Slide title text"],
        ),
    ];
    // One worker for all of them: each is made after the one before.
    let mut host = host();
    for (name, kind, expected) in cases {
        let made = create(&mut host, &[(&corpus(name), kind)])
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(made.pages >= 1, "{name}");
        assert!(made.bytes.starts_with(b"%PDF-"), "{name}");
        let text = words(&mut host, &made);
        for word in expected {
            assert!(text.contains(word), "{name}: {word:?} not in {text:?}");
        }
        // The page says nothing of who made the source, what it is called, or where it points.
        for private in [
            "Private-Author",
            "Private title",
            "beacon.example.invalid",
            "SCRIPT RAN",
            "remote picture",
        ] {
            assert!(!text.contains(private), "{name}: {private} in {text:?}");
            assert!(!contains(&made.bytes, private.as_bytes()), "{name}");
        }
        assert!(!contains(&made.bytes, b"/Info"), "{name}");
        assert!(!contains(&made.bytes, b"/URI"), "{name}");
        assert!(!contains(&made.bytes, b"/JavaScript"), "{name}");
        assert!(!contains(&made.bytes, b"/OpenAction"), "{name}");
    }
}

#[test]
fn pictures_go_together_a_page_each_and_a_camera_leaves_nothing() {
    let mut host = host();
    let jpeg = corpus("images/stamp-exif.jpg");
    let png = corpus("images/stamp-metadata.png");
    let made = create(
        &mut host,
        &[
            (&jpeg, CreateKind::Image),
            (&png, CreateKind::Image),
            (&jpeg, CreateKind::Image),
        ],
    )
    .expect("three pictures");
    assert_eq!(made.pages, 3);
    for private in [&b"Canon"[..], b"2023:07:04", b"Exif", b"Private-Author"] {
        assert!(!contains(&made.bytes, private));
    }
    assert!(!contains(&made.bytes, b"/Info"));
    // They are pages of a PDF the worker opens.
    words(&mut host, &made);
}

#[test]
fn a_picture_and_a_document_do_not_go_together_and_the_worker_goes_on() {
    let mut host = host();
    let refused = create(
        &mut host,
        &[
            (&corpus("images/stamp-exif.jpg"), CreateKind::Image),
            (&corpus("sources/text.txt"), CreateKind::Text),
        ],
    )
    .err()
    .expect("a picture and a text");
    assert_eq!(code(&refused), ErrorCode::InvalidArgument);
    let refused = create(
        &mut host,
        &[
            (&corpus("sources/text.txt"), CreateKind::Text),
            (&corpus("sources/book.epub"), CreateKind::Epub),
        ],
    )
    .err()
    .expect("a text and a book");
    assert_eq!(code(&refused), ErrorCode::InvalidArgument);
    // No file, and too many files.
    assert!(create(&mut host, &[]).is_err());
    let one = corpus("images/stamp-exif.jpg");
    let many: Vec<(&Path, CreateKind)> = (0..=MAX_CREATE_SOURCES)
        .map(|_| (one.as_path(), CreateKind::Image))
        .collect();
    let refused = create(&mut host, &many).err().expect("too many");
    assert_eq!(code(&refused), ErrorCode::InvalidArgument);
    assert!(host.is_running());
    let made = create(
        &mut host,
        &[(&corpus("sources/text.txt"), CreateKind::Text)],
    )
    .expect("the worker goes on");
    assert_eq!(made.pages, 1);
}

#[test]
fn a_file_that_is_not_what_it_says_is_an_error_and_the_worker_goes_on() {
    let mut host = host();
    let scratch = temp_path("scratch");
    let cases: [(&[u8], CreateKind); 7] = [
        (b"not a zip at all", CreateKind::Docx),
        (b"not a zip at all", CreateKind::Epub),
        (b"not a picture either", CreateKind::Image),
        (b"", CreateKind::Text),
        (&[0xFF, 0xFE, 0x00, 0xD8], CreateKind::Text),
        (b"   \n\n", CreateKind::Text),
        (b"<svg", CreateKind::Svg),
    ];
    for (bytes, kind) in cases {
        std::fs::write(&scratch, bytes).expect("the scratch file");
        let refused = create(&mut host, &[(&scratch, kind)]).err();
        let refused = refused.unwrap_or_else(|| panic!("{kind:?}: {bytes:?} made a PDF"));
        assert!(
            matches!(
                code(&refused),
                ErrorCode::Corrupted | ErrorCode::InvalidArgument
            ),
            "{kind:?}: {refused}"
        );
        assert!(host.is_running(), "{kind:?}");
    }
    std::fs::remove_file(&scratch).ok();
    // A picture that says it is 60000 x 60000 pixels is not decoded.
    let refused = create(
        &mut host,
        &[(
            &corpus("images/stamp-huge-dimensions.png"),
            CreateKind::Image,
        )],
    )
    .err()
    .expect("a huge picture");
    assert!(
        matches!(
            code(&refused),
            ErrorCode::LimitExceeded | ErrorCode::Corrupted
        ),
        "{refused}"
    );
    let made = create(
        &mut host,
        &[(&corpus("sources/picture.svg"), CreateKind::Svg)],
    )
    .expect("the worker goes on");
    assert_eq!(made.pages, 1);
}

#[test]
fn a_file_larger_than_a_source_may_be_is_refused_after_the_limit_and_no_more_is_read() {
    let mut host = host();
    let big = temp_path("big.txt");
    let file = File::create(&big).expect("create");
    file.set_len(MAX_SOURCE_BYTES as u64 + 1).expect("sparse");
    drop(file);
    let refused = create(&mut host, &[(&big, CreateKind::Text)])
        .err()
        .expect("too large");
    std::fs::remove_file(&big).ok();
    assert_eq!(code(&refused), ErrorCode::LimitExceeded);
    assert!(host.is_running());
}

/// A bit string, as the deflate format writes it: values least significant bit first, Huffman
/// codes most significant bit first.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    pending: u64,
    count: u32,
}

impl Bits {
    fn put(&mut self, value: u32, count: u32) {
        self.pending |= u64::from(value) << self.count;
        self.count += count;
        while self.count >= 8 {
            self.bytes.push(self.pending as u8);
            self.pending >>= 8;
            self.count -= 8;
        }
    }

    fn code(&mut self, code: u32, length: u32) {
        let mut reversed = 0;
        for bit in 0..length {
            reversed |= ((code >> bit) & 1) << (length - 1 - bit);
        }
        self.put(reversed, length);
    }

    /// A letter of the fixed code (RFC 1951, 3.2.6), under 128.
    fn letter(&mut self, byte: u8) {
        self.code(0x30 + u32::from(byte), 8);
    }

    /// 258 letters, the same as the one before.
    fn repeat_258(&mut self) {
        self.code(0xC5, 8);
        self.code(0, 5);
    }

    fn finish(mut self) -> Vec<u8> {
        self.code(0, 7);
        if self.count > 0 {
            self.bytes.push(self.pending as u8);
        }
        self.bytes
    }
}

/// A ZIP archive of the parts `(name, deflated, bytes as stored, size once unpacked)`.
fn zip(parts: &[(&str, bool, &[u8], u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, deflated, data, size) in parts {
        let method: u16 = if *deflated { 8 } else { 0 };
        let offset = out.len() as u32;
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0x21, 0]);
        out.extend_from_slice(&0u32.to_le_bytes()); // MuPDF does not check the checksum
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        directory.extend_from_slice(&method.to_le_bytes());
        directory.extend_from_slice(&[0, 0, 0x21, 0]);
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&(data.len() as u32).to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 12]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let start = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// A Word file whose text is one word and then `repeats` times 258 letters, packed as small as
/// the format goes (13 bits for 258 bytes): about 1.5 GiB for six million. Every part but the
/// text is what a Word file has to have for MuPDF to read it.
fn word_file_of_a_very_long_word(repeats: u32) -> (Vec<u8>, u64) {
    const HEAD: &[u8] = b"<?xml version=\"1.0\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>";
    const TAIL: &[u8] = b"</w:t></w:r></w:p></w:body></w:document>";
    let mut bits = Bits::default();
    bits.put(1, 1); // the last block
    bits.put(1, 2); // with the fixed codes
    for &byte in HEAD {
        bits.letter(byte);
    }
    bits.letter(b'A');
    for _ in 0..repeats {
        bits.repeat_258();
    }
    for &byte in TAIL {
        bits.letter(byte);
    }
    let packed = bits.finish();
    let size = (HEAD.len() + 1 + 258 * repeats as usize + TAIL.len()) as u64;
    let content_types: &[u8] = b"<?xml version=\"1.0\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>";
    let rels: &[u8] = b"<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/></Relationships>";
    let none: &[u8] = b"<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>";
    let file = zip(&[
        (
            "[Content_Types].xml",
            false,
            content_types,
            content_types.len() as u32,
        ),
        ("_rels/.rels", false, rels, rels.len() as u32),
        (
            "word/_rels/document.xml.rels",
            false,
            none,
            none.len() as u32,
        ),
        ("word/document.xml", true, &packed, size as u32),
    ]);
    (file, size)
}

#[test]
fn a_word_file_that_unpacks_to_more_than_the_worker_may_hold_ends_in_an_error_not_in_the_end_of_the_app()
 {
    let mut host = host();
    // The small one first: the same kind of file, a few words, is made.
    let (small, size) = word_file_of_a_very_long_word(2);
    assert!(size > 516 && size < 1_000);
    let path = temp_path("small.docx");
    std::fs::write(&path, &small).expect("write");
    let made = create(&mut host, &[(&path, CreateKind::Docx)]).expect("a small word");
    assert!(made.pages >= 1);
    drop(made);

    // Six million times 258 letters: a file of 10 MB that is 1.5 GiB when unpacked, which the
    // worker, held to 2 GiB, cannot also read as text. The request ends, in an error.
    let (bomb, size) = word_file_of_a_very_long_word(6_000_000);
    assert!(size > 1_500_000_000 && size < u64::from(u32::MAX));
    assert!(bomb.len() < 11_000_000, "{}", bomb.len());
    std::fs::write(&path, &bomb).expect("write");
    let started = std::time::Instant::now();
    let outcome = create(&mut host, &[(&path, CreateKind::Docx)]);
    let took = started.elapsed();
    std::fs::remove_file(&path).ok();
    assert!(
        took < std::time::Duration::from_secs(120),
        "took {took:?}: {:?}",
        outcome.as_ref().err()
    );
    assert!(
        outcome.is_err(),
        "a PDF of 1.5 GiB of one word was made: {} pages",
        outcome.as_ref().map_or(0, |made| made.pages)
    );
    // Whatever became of the worker, the next request is answered: it is started again if it
    // was ended.
    let made = create(
        &mut host,
        &[(&corpus("sources/text.txt"), CreateKind::Text)],
    )
    .expect("the worker goes on");
    assert_eq!(made.pages, 1);
}

#[test]
fn a_word_file_that_says_it_is_3_gib_unpacked_does_not_make_the_worker_hold_that_much() {
    let mut host = host();
    let outcome = create(
        &mut host,
        &[(&corpus("sources/bomb-declared-size.docx"), CreateKind::Docx)],
    );
    // The room it asks for is more than the worker may hold: an error. (Were the limit higher,
    // the few words it really has would be made.)
    assert!(outcome.is_err(), "{:?}", outcome.map(|made| made.pages));
    let made = create(
        &mut host,
        &[(&corpus("sources/text.txt"), CreateKind::Text)],
    )
    .expect("the worker goes on");
    assert_eq!(made.pages, 1);
}
