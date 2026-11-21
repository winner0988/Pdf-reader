//! A picture for a custom stamp, through the real worker in the real sandbox (B2-08): the worker
//! reads it through a read-only handle (never a path), decodes it, and gives back the pixels
//! alone as a PNG; that PNG then becomes a stamp of a document, which is saved. Nothing a camera
//! wrote besides the pixels reaches the saved file.
#![cfg(windows)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ipc_contract::limits::MAX_STAMP_SIDE_PX;
use ipc_contract::types::{AnnotationKind, DocumentId, Rect, Rotation};
use ipc_contract::validate::stamp_png_size;
use ipc_contract::worker::{WorkerEdit, WorkerErrorCode, WorkerRequest, WorkerResponse};
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

/// What the worker makes of the picture in the corpus file `name`.
fn prepared(host: &mut WorkerHost, name: &str) -> Result<(Vec<u8>, u32, u32), HostError> {
    let file = File::open(corpus(name)).expect("the sample");
    match host.prepare_stamp_image(&file)? {
        WorkerResponse::StampImage {
            png, width, height, ..
        } => Ok((png, width, height)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_picture_with_exif_becomes_a_stamp_in_a_saved_document_without_it() {
    let mut host = host();
    let (png, width, height) = prepared(&mut host, "images/stamp-exif.jpg").expect("prepared");
    assert_eq!((width, height), (64, 32));
    assert_eq!(stamp_png_size(&png), Ok((64, 32)));
    assert!(png.len() < 20_000);
    for private in [&b"Canon"[..], b"2023:07:04", b"Exif"] {
        assert!(!contains(&png, private));
    }

    let (doc, _) = match host.open(&corpus("benign/single-page.pdf")).expect("open") {
        (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
        other => panic!("{other:?}"),
    };
    let stamped = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddImageStamp {
                page: 0,
                rect: Rect {
                    x0: 100.0,
                    y0: 100.0,
                    x1: 228.0,
                    y1: 164.0,
                },
                png: png.clone(),
            },
        })
        .expect("the stamp");
    assert!(
        matches!(stamped, WorkerResponse::Edited { .. }),
        "{stamped:?}"
    );
    let listed = host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc,
            page_index: 0,
        })
        .expect("annotations");
    let WorkerResponse::PageAnnotations { annotations, .. } = listed else {
        panic!("{listed:?}");
    };
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].kind, AnnotationKind::Stamp);

    let path = std::env::temp_dir().join(format!("pdf-worker-{}-stamp.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    host.save(doc, &file).expect("save");
    drop(file);
    let saved = std::fs::read(&path).expect("saved file");
    std::fs::remove_file(&path).ok();
    assert!(saved.starts_with(b"%PDF"));
    for private in [&b"Canon"[..], b"2023:07:04", b"Exif", b"Mark IV"] {
        assert!(!contains(&saved, private));
    }
}

#[test]
fn a_picture_stamp_is_turned_in_the_real_worker_and_comes_back_turned_from_the_saved_file() {
    let mut host = host();
    let (png, ..) = prepared(&mut host, "images/stamp-exif.jpg").expect("prepared");
    let doc = match host.open(&corpus("benign/single-page.pdf")).expect("open") {
        (doc, WorkerResponse::Opened { .. }) => doc,
        other => panic!("{other:?}"),
    };
    let edit = |host: &mut WorkerHost, edit: WorkerEdit| {
        let done = host
            .request(|request| WorkerRequest::Edit { request, doc, edit })
            .expect("the edit");
        assert!(matches!(done, WorkerResponse::Edited { .. }), "{done:?}");
    };
    let listed = |host: &mut WorkerHost| {
        let listed = host
            .request(|request| WorkerRequest::GetPageAnnotations {
                request,
                doc,
                page_index: 0,
            })
            .expect("annotations");
        let WorkerResponse::PageAnnotations { annotations, .. } = listed else {
            panic!("{listed:?}");
        };
        annotations
    };
    edit(
        &mut host,
        WorkerEdit::AddImageStamp {
            page: 0,
            rect: Rect {
                x0: 100.0,
                y0: 100.0,
                x1: 228.0,
                y1: 164.0,
            },
            png,
        },
    );
    let [stamp] = &listed(&mut host)[..] else {
        panic!("one stamp");
    };
    assert_eq!(stamp.picture.expect("a picture").angle, 0.0);
    edit(
        &mut host,
        WorkerEdit::SetPictureAngle {
            page: 0,
            annotation: stamp.id,
            degrees: 90.0,
        },
    );
    let [turned] = &listed(&mut host)[..] else {
        panic!("one stamp");
    };
    // A quarter turned: 64 across and 128 down, about the same middle.
    assert!(
        ((turned.rect.x1 - turned.rect.x0) - 64.0).abs() < 0.1,
        "{turned:?}"
    );
    assert!(
        ((turned.rect.y1 - turned.rect.y0) - 128.0).abs() < 0.1,
        "{turned:?}"
    );
    assert!((turned.picture.expect("a picture").angle - 90.0).abs() < 0.01);

    // Written, and read again by another worker: still turned.
    let path = std::env::temp_dir().join(format!("pdf-worker-{}-turned.pdf", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    host.save(doc, &file).expect("save");
    drop(file);
    let mut other = self::host();
    let (again, _) = other.open(&path).expect("open the copy");
    let listed = other
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc: again,
            page_index: 0,
        })
        .expect("annotations");
    std::fs::remove_file(&path).ok();
    let WorkerResponse::PageAnnotations { annotations, .. } = listed else {
        panic!("{listed:?}");
    };
    let [stamp] = &annotations[..] else {
        panic!("one stamp");
    };
    assert!((stamp.picture.expect("a picture").angle - 90.0).abs() < 0.01);
}

#[test]
fn a_png_picture_is_made_too_without_its_chunks() {
    let mut host = host();
    let (png, width, height) = prepared(&mut host, "images/stamp-metadata.png").expect("prepared");
    assert_eq!((width, height), (64, 32));
    assert!(!contains(&png, b"Private-Author"));
    assert!(!contains(&png, b"eXIf"));
    assert!(!contains(&png, b"tEXt"));
    assert!(width.max(height) <= MAX_STAMP_SIDE_PX);
}

#[test]
fn what_is_not_a_usable_picture_is_refused_and_the_worker_goes_on() {
    let mut host = host();
    // A PNG of 60000 x 60000 pixels, said in its header: refused, not decoded.
    let huge = prepared(&mut host, "images/stamp-huge-dimensions.png");
    assert!(huge.is_err(), "{huge:?}");
    assert!(host.is_running());
    // A document is no picture.
    let not_a_picture = prepared(&mut host, "benign/single-page.pdf");
    assert!(not_a_picture.is_err(), "{not_a_picture:?}");
    // The worker still serves.
    assert!(prepared(&mut host, "images/stamp-exif.jpg").is_ok());
    // The error the host gives is the worker's code for it.
    let error = prepared(&mut host, "benign/single-page.pdf").unwrap_err();
    assert!(
        matches!(
            error,
            HostError::Worker(ref worker) if worker.code == WorkerErrorCode::InvalidRequest
        ),
        "{error:?}"
    );
}

#[test]
fn a_stamp_edit_with_something_else_than_a_stamp_picture_is_refused() {
    let mut host = host();
    let (doc, _): (DocumentId, _) =
        match host.open(&corpus("benign/single-page.pdf")).expect("open") {
            (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
            other => panic!("{other:?}"),
        };
    let jpeg = std::fs::read(corpus("images/stamp-exif.jpg")).expect("the sample");
    for wrong in [Vec::new(), jpeg] {
        let refused = host.request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddImageStamp {
                page: 0,
                rect: Rect {
                    x0: 10.0,
                    y0: 10.0,
                    x1: 90.0,
                    y1: 50.0,
                },
                png: wrong.clone(),
            },
        });
        assert!(refused.is_err(), "{refused:?}");
    }
    assert!(host.is_running());
}

fn make_transparent_line_png(width: u32, height: u32) -> Vec<u8> {
    let mut raw_scanlines = Vec::new();
    let mid_y = height / 2;
    for y in 0..height {
        raw_scanlines.push(0u8); // filter type none
        let is_line = y == mid_y || (height > 1 && y == mid_y - 1);
        for _x in 0..width {
            if is_line {
                raw_scanlines.extend_from_slice(&[0, 0, 0, 255]); // opaque black
            } else {
                raw_scanlines.extend_from_slice(&[0, 0, 0, 0]); // transparent
            }
        }
    }

    let mut zlib = vec![0x78, 0x01];
    let len = raw_scanlines.len();
    assert!(len <= 65535, "length fits in u16");
    let len_u16 = len as u16;
    let nlen_u16 = !len_u16;
    zlib.push(0x01); // BFINAL=1, BTYPE=00 (uncompressed)
    zlib.extend_from_slice(&len_u16.to_le_bytes());
    zlib.extend_from_slice(&nlen_u16.to_le_bytes());
    zlib.extend_from_slice(&raw_scanlines);

    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for b in &raw_scanlines {
        s1 = (s1 + *b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    let adler = (s2 << 16) | s1;
    zlib.extend_from_slice(&adler.to_be_bytes());

    let mut png = Vec::new();
    png.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_png_chunk(&mut png, b"IHDR", &ihdr);
    write_png_chunk(&mut png, b"IDAT", &zlib);
    write_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::new();
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&calc_crc32(&crc_input).to_be_bytes());
}

fn calc_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDBA_7720 & mask);
        }
    }
    !crc
}

#[test]
fn a_transparent_png_stamp_preserves_page_color_and_draws_lines_dark() {
    let mut host = host();
    let png = make_transparent_line_png(32, 32);
    assert_eq!(stamp_png_size(&png), Ok((32, 32)));

    let (doc, _) = match host.open(&corpus("benign/single-page.pdf")).expect("open") {
        (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
        other => panic!("{other:?}"),
    };

    let rect = Rect {
        x0: 100.0,
        y0: 100.0,
        x1: 200.0,
        y1: 200.0,
    };
    let stamped = host
        .request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddImageStamp {
                page: 0,
                rect,
                png: png.clone(),
            },
        })
        .expect("the stamp");
    assert!(matches!(stamped, WorkerResponse::Edited { .. }));

    let path = std::env::temp_dir().join(format!("transparent-stamp-{}.pdf", std::process::id()));
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&path)
        .expect("temp file");
    let saved = host.save(doc, &file).expect("save");
    drop(file);
    assert!(matches!(saved, WorkerResponse::Saved { .. }));

    let mut other = (self::host)();
    let (again, _) = other.open(&path).expect("open copy");
    let rendered = other
        .request(|request| WorkerRequest::Render {
            request,
            doc: again,
            page_index: 0,
            scale: 1.0,
            rotation: Rotation::None,
        })
        .expect("render");
    std::fs::remove_file(&path).ok();

    let WorkerResponse::Rendered { raster, .. } = rendered else {
        panic!("expected Rendered, got {rendered:?}");
    };

    let pixel_at = |x: u32, y: u32| -> [u8; 4] {
        let offset = ((y * raster.width + x) * 4) as usize;
        [
            raster.pixels[offset],
            raster.pixels[offset + 1],
            raster.pixels[offset + 2],
            raster.pixels[offset + 3],
        ]
    };

    // Transparent area shows white page background
    let transparent_pixel = pixel_at(150, 110);
    assert!(
        transparent_pixel[0] > 240 && transparent_pixel[1] > 240 && transparent_pixel[2] > 240,
        "transparent area should be white page: {transparent_pixel:?}"
    );

    // Line area is dark
    let line_pixel = pixel_at(150, 150);
    assert!(
        line_pixel[0] < 50 && line_pixel[1] < 50 && line_pixel[2] < 50,
        "line area should be dark: {line_pixel:?}"
    );
}
