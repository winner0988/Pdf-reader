//! #440 的 POC，不合併。
//!
//! Uses `pdf_worker::engine::PdfDocument` (the same engine code the app uses) to verify:
//! - POC 1: merging 3 sources with 3-level outlines (with and without per-source bookmarks);
//! - POC 3: stripping outline actions (`/A`, `/Next` chains, `/JS`, `/Launch`, `/GoToR`, `/URI`,
//!   `/OpenAction`, `/AA`), handling `outline-cycle.pdf`, depth limits, and `MAX_OUTLINE_ITEMS`;
//! - POC 4: layer detection (`has_layers`) and flattening `/OCProperties` so hidden layers render.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use ipc_contract::limits::{MAX_OUTLINE_DEPTH, MAX_OUTLINE_ITEMS};
use mupdf::pdf::PdfDocument as MuPdfDocument;
use pdf_worker::engine::{
    DocumentOutline, MergedOutlineEntry, OutlineTarget, PdfDocument, SourceOutlineInput,
    build_merged_outline,
};
use pdf_worker::scan::ScanBudget;

fn corpus(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn poc_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("poc-440");
    std::fs::create_dir_all(&dir).expect("create poc-440 dir");
    dir
}

fn pdf_string(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)");
    format!("({escaped})")
}

fn utf16_hex(text: &str) -> String {
    let units: String = text
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect();
    format!("<FEFF{units}>")
}

struct OutlineSpec<'a> {
    title_pdf: String,
    depth: usize,
    page: Option<usize>,
    extra: &'a str,
}

/// Builds a multi-page PDF with Helvetica page labels and an outline tree (like `generate.py`).
fn build_pdf_with_outline(
    page_texts: &[&str],
    entries: &[OutlineSpec<'_>],
    catalog_extra: &str,
    page_extra: &str,
) -> Vec<u8> {
    let pages = page_texts.len();
    assert!(pages > 0);
    let mut objects: Vec<String> = Vec::new();
    let reserve = |objs: &mut Vec<String>| -> usize {
        objs.push(String::new());
        objs.len()
    };

    let catalog = reserve(&mut objects); // 1
    let pages_root = reserve(&mut objects); // 2
    let font = reserve(&mut objects); // 3
    objects[font - 1] =
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned();

    let mut page_nums = Vec::with_capacity(pages);
    for _ in 0..pages {
        page_nums.push(reserve(&mut objects));
    }
    for (idx, text) in page_texts.iter().enumerate() {
        let ops = format!("BT /F1 24 Tf 72 720 Td {} Tj ET\n", pdf_string(text));
        let stream_num = reserve(&mut objects);
        objects[stream_num - 1] = format!("<< /Length {} >>\nstream\n{ops}\nendstream", ops.len());
        objects[page_nums[idx] - 1] = format!(
            "<< /Type /Page /Parent {pages_root} 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {stream_num} 0 R{page_extra} >>"
        );
    }

    let kids: Vec<String> = page_nums.iter().map(|n| format!("{n} 0 R")).collect();
    objects[pages_root - 1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {pages} >>",
        kids.join(" ")
    );

    let outlines_ref = if entries.is_empty() {
        String::new()
    } else {
        let root = reserve(&mut objects);
        let nums: Vec<usize> = (0..entries.len()).map(|_| reserve(&mut objects)).collect();
        let mut parents: Vec<usize> = Vec::new();
        let mut children: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        children.insert(root, Vec::new());

        for (i, entry) in entries.iter().enumerate() {
            parents.truncate(entry.depth);
            let parent = parents.last().map_or(root, |&p| nums[p]);
            children.entry(parent).or_default().push(i);
            parents.push(i);
        }

        for (&parent, kids_idx) in &children {
            for (k, &i) in kids_idx.iter().enumerate() {
                let entry = &entries[i];
                let mut dict = format!("<< /Title {} /Parent {parent} 0 R", entry.title_pdf);
                if k > 0 {
                    dict.push_str(&format!(" /Prev {} 0 R", nums[kids_idx[k - 1]]));
                }
                if k + 1 < kids_idx.len() {
                    dict.push_str(&format!(" /Next {} 0 R", nums[kids_idx[k + 1]]));
                }
                if let Some(own) = children.get(&nums[i])
                    && !own.is_empty()
                {
                    dict.push_str(&format!(
                        " /First {} 0 R /Last {} 0 R /Count {}",
                        nums[own[0]],
                        nums[own[own.len() - 1]],
                        own.len()
                    ));
                }
                if let Some(page) = entry.page {
                    dict.push_str(&format!(" /Dest [{} 0 R /Fit]", page_nums[page]));
                }
                if !entry.extra.is_empty() {
                    dict.push(' ');
                    dict.push_str(entry.extra);
                }
                dict.push_str(" >>");
                objects[nums[i] - 1] = dict;
            }
        }
        let top = &children[&root];
        objects[root - 1] = format!(
            "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>",
            nums[top[0]],
            nums[top[top.len() - 1]],
            top.len()
        );
        format!(" /Outlines {root} 0 R")
    };

    objects[catalog - 1] =
        format!("<< /Type /Catalog /Pages {pages_root} 0 R{outlines_ref}{catalog_extra} >>");
    common::build_pdf(&objects)
}

struct PreparedSourceInput {
    title: String,
    bytes: Vec<u8>,
    pages: u32,
    outline: DocumentOutline,
    has_layers: bool,
}

fn prepare_for_merge(title: &str, raw_bytes: &[u8]) -> PreparedSourceInput {
    let prepared = PdfDocument::prepare_source(raw_bytes, None).expect("prepare_source");
    let plain = PdfDocument::from_bytes(&prepared.bytes).expect("open prepared plain copy");
    let outline = plain
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .expect("read source outline");
    PreparedSourceInput {
        title: title.to_owned(),
        bytes: prepared.bytes,
        pages: prepared.pages,
        outline,
        has_layers: prepared.has_layers,
    }
}

fn merge_sources(
    sources: &[PreparedSourceInput],
    include_source_bookmark: bool,
) -> (PdfDocument, Vec<u8>) {
    let mut merged = PdfDocument::new_blank().expect("blank document");
    let mut outline_inputs: Vec<SourceOutlineInput<'_>> = Vec::with_capacity(sources.len());
    let mut start_pages = Vec::with_capacity(sources.len());

    for source in sources {
        let start_page = merged.page_count().expect("page count");
        merged
            .insert_pages(start_page, &source.bytes, &[])
            .expect("insert_pages");
        start_pages.push(start_page);
    }

    for (source, &start_page) in sources.iter().zip(&start_pages) {
        outline_inputs.push(SourceOutlineInput {
            title: &source.title,
            start_page,
            page_count: source.pages,
            outline: &source.outline,
        });
    }

    let entries: Vec<MergedOutlineEntry> =
        build_merged_outline(&outline_inputs, include_source_bookmark, "（沒有標題）");
    merged.write_outline(&entries).expect("write_outline");

    let mut out = Vec::new();
    merged.write_merged(&mut out).expect("write_merged");
    (merged, out)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Checks `/Outlines` links (`/Parent`, `/First`, `/Last`, `/Prev`, `/Next`, `/Count`) and asserts
/// that every item with children is collapsed (`/Count < 0`) and has no `/A`, `/C`, `/F`, or `/SE`.
fn assert_collapsed_outline_links(pdf_bytes: &[u8], expected_root_count: Option<i32>) {
    let doc = MuPdfDocument::from_bytes(pdf_bytes).expect("open mupdf");
    let catalog = doc.catalog().expect("catalog");
    let root = catalog.get_dict("Outlines").expect("Outlines");
    let Some(root) = root else {
        assert_eq!(expected_root_count, None);
        return;
    };
    let root_count = root
        .get_dict("Count")
        .expect("root count")
        .map(|c| c.as_int().expect("int"));
    assert_eq!(root_count, expected_root_count);

    fn walk_under(parent: &mupdf::pdf::PdfObject) -> i32 {
        let parent_num = parent.as_indirect().expect("indirect");
        let mut children = Vec::new();
        let mut visible_count = 0;
        let mut curr = parent.get_dict("First").expect("First");
        while let Some(item) = curr {
            let num = item.as_indirect().expect("item indirect");
            assert_eq!(
                item.get_dict("Parent")
                    .expect("Parent")
                    .expect("has Parent")
                    .as_indirect()
                    .expect("parent indirect"),
                parent_num
            );
            assert_eq!(
                item.get_dict("Prev")
                    .expect("Prev")
                    .map(|p| p.as_indirect().expect("prev indirect")),
                children.last().copied()
            );
            // No action, style, color, or structure element keys.
            for forbidden in ["A", "C", "F", "SE"] {
                assert!(
                    item.get_dict(forbidden).expect(forbidden).is_none(),
                    "outline item {num} unexpectedly has /{forbidden}"
                );
            }
            let direct_or_visible_under = walk_under(&item);
            let count = item
                .get_dict("Count")
                .expect("Count")
                .map(|c| c.as_int().expect("int"));
            if item.get_dict("First").expect("First").is_some() {
                // Collapsed: negative count whose magnitude equals visible children when opened.
                assert_eq!(count, Some(-direct_or_visible_under));
            } else {
                assert_eq!(count, None);
            }
            // Because every item with children is closed, this item contributes 1 visible row.
            visible_count += 1;
            children.push(num);
            curr = item.get_dict("Next").expect("Next");
        }
        assert_eq!(
            parent
                .get_dict("Last")
                .expect("Last")
                .map(|l| l.as_indirect().expect("last indirect")),
            children.last().copied()
        );
        visible_count
    }

    let top_visible = walk_under(&root);
    assert_eq!(Some(top_visible), expected_root_count);
}

#[test]
fn poc_1_three_sources_three_level_outline_with_and_without_source_bookmarks() {
    let source_1_bytes = corpus("benign/outline-3-levels.pdf");
    let source_2_bytes = build_pdf_with_outline(
        &[
            "Source 2 - Page 1: Part A",
            "Source 2 - Page 2: Topic A.1",
            "Source 2 - Page 3: Detail A.1.a",
            "Source 2 - Page 4: Topic A.2 & Section B",
        ],
        &[
            OutlineSpec {
                title_pdf: pdf_string("Part A"),
                depth: 0,
                page: Some(0),
                extra: "",
            },
            OutlineSpec {
                title_pdf: pdf_string("Topic A.1"),
                depth: 1,
                page: Some(1),
                extra: "",
            },
            OutlineSpec {
                title_pdf: pdf_string("Detail A.1.a"),
                depth: 2,
                page: Some(2),
                extra: "",
            },
            OutlineSpec {
                title_pdf: pdf_string("Topic A.2"),
                depth: 1,
                page: Some(3),
                extra: "",
            },
            OutlineSpec {
                title_pdf: utf16_hex("中文第二部分"),
                depth: 0,
                page: Some(3),
                extra: "",
            },
        ],
        "",
        "",
    );
    let source_3_bytes = build_pdf_with_outline(
        &[
            "Source 3 - Page 1: Appendix I",
            "Source 3 - Page 2: Table I.1",
            "Source 3 - Page 3: Appendix II & Notes",
        ],
        &[
            OutlineSpec {
                title_pdf: pdf_string("Appendix I"),
                depth: 0,
                page: Some(0),
                extra: "",
            },
            OutlineSpec {
                title_pdf: pdf_string("Table I.1"),
                depth: 1,
                page: Some(1),
                extra: "",
            },
            OutlineSpec {
                title_pdf: pdf_string("Appendix II"),
                depth: 0,
                page: Some(2),
                extra: "",
            },
            OutlineSpec {
                title_pdf: pdf_string("Notes"),
                depth: 0,
                page: Some(2),
                extra: "",
            },
        ],
        "",
        "",
    );

    let sources = vec![
        prepare_for_merge("outline-3-levels", &source_1_bytes),
        prepare_for_merge("source-2-handbook", &source_2_bytes),
        prepare_for_merge("source-3-appendix", &source_3_bytes),
    ];

    // Variant A: with per-source top-level bookmarks.
    let (merged_with, bytes_with) = merge_sources(&sources, true);
    assert_eq!(merged_with.page_count().unwrap(), 13);
    let expected_with = vec![
        ("outline-3-levels", 0u16, OutlineTarget::Page(0)),
        ("Chapter 1", 1, OutlineTarget::Page(0)),
        ("Section 1.1", 2, OutlineTarget::Page(1)),
        ("Subsection 1.1.1", 3, OutlineTarget::Page(2)),
        ("Chapter 2", 1, OutlineTarget::Page(3)),
        ("Section 2.1", 2, OutlineTarget::Page(4)),
        ("Appendix", 1, OutlineTarget::Page(5)),
        ("source-2-handbook", 0, OutlineTarget::Page(6)),
        ("Part A", 1, OutlineTarget::Page(6)),
        ("Topic A.1", 2, OutlineTarget::Page(7)),
        ("Detail A.1.a", 3, OutlineTarget::Page(8)),
        ("Topic A.2", 2, OutlineTarget::Page(9)),
        ("中文第二部分", 1, OutlineTarget::Page(9)),
        ("source-3-appendix", 0, OutlineTarget::Page(10)),
        ("Appendix I", 1, OutlineTarget::Page(10)),
        ("Table I.1", 2, OutlineTarget::Page(11)),
        ("Appendix II", 1, OutlineTarget::Page(12)),
        ("Notes", 1, OutlineTarget::Page(12)),
    ];

    let simplify = |doc: &PdfDocument| -> Vec<(String, u16, OutlineTarget)> {
        let outline = doc
            .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
            .expect("outline");
        assert!(!outline.truncated);
        outline
            .entries
            .into_iter()
            .map(|e| (e.title, e.depth, e.target))
            .collect()
    };
    let expected_owned: Vec<(String, u16, OutlineTarget)> = expected_with
        .iter()
        .map(|(t, d, p)| ((*t).to_owned(), *d, p.clone()))
        .collect();
    assert_eq!(simplify(&merged_with), expected_owned);

    let out_with = poc_dir().join("poc1-with-source-bookmarks.pdf");
    std::fs::write(&out_with, &bytes_with).expect("write poc1-with-source-bookmarks.pdf");
    common::check_qpdf(&out_with);
    assert!(!contains(&bytes_with, b"/Info") && !contains(&bytes_with, b"/Producer"));
    assert_collapsed_outline_links(&bytes_with, Some(3));

    let mut reopened_with = PdfDocument::from_bytes(&bytes_with).expect("reopen with");
    assert_eq!(reopened_with.page_count().unwrap(), 13);
    assert_eq!(simplify(&reopened_with), expected_owned);

    // Merged outline remains editable with normal bookmark editing APIs, and survives save + reopen.
    let first_id = reopened_with
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap()
        .entries[0]
        .id;
    reopened_with
        .rename_bookmark(first_id, "outline-3-levels-edited")
        .expect("rename merged bookmark");
    let mut resaved = Vec::new();
    reopened_with.save(&mut resaved).expect("save edited");
    let reopened_after_save = PdfDocument::from_bytes(&resaved).expect("reopen after save");
    let after_edit = simplify(&reopened_after_save);
    assert_eq!(after_edit[0].0, "outline-3-levels-edited");
    assert_eq!(after_edit.len(), expected_owned.len());
    // Collapsed state of other bookmarks stays intact after editing.
    assert_collapsed_outline_links(&resaved, Some(3));

    // Variant B: without per-source top-level bookmarks.
    let (merged_without, bytes_without) = merge_sources(&sources, false);
    assert_eq!(merged_without.page_count().unwrap(), 13);
    let expected_without: Vec<(String, u16, OutlineTarget)> = vec![
        ("Chapter 1".to_owned(), 0, OutlineTarget::Page(0)),
        ("Section 1.1".to_owned(), 1, OutlineTarget::Page(1)),
        ("Subsection 1.1.1".to_owned(), 2, OutlineTarget::Page(2)),
        ("Chapter 2".to_owned(), 0, OutlineTarget::Page(3)),
        ("Section 2.1".to_owned(), 1, OutlineTarget::Page(4)),
        ("Appendix".to_owned(), 0, OutlineTarget::Page(5)),
        ("Part A".to_owned(), 0, OutlineTarget::Page(6)),
        ("Topic A.1".to_owned(), 1, OutlineTarget::Page(7)),
        ("Detail A.1.a".to_owned(), 2, OutlineTarget::Page(8)),
        ("Topic A.2".to_owned(), 1, OutlineTarget::Page(9)),
        ("中文第二部分".to_owned(), 0, OutlineTarget::Page(9)),
        ("Appendix I".to_owned(), 0, OutlineTarget::Page(10)),
        ("Table I.1".to_owned(), 1, OutlineTarget::Page(11)),
        ("Appendix II".to_owned(), 0, OutlineTarget::Page(12)),
        ("Notes".to_owned(), 0, OutlineTarget::Page(12)),
    ];
    assert_eq!(simplify(&merged_without), expected_without);

    let out_without = poc_dir().join("poc1-without-source-bookmarks.pdf");
    std::fs::write(&out_without, &bytes_without).expect("write poc1-without-source-bookmarks.pdf");
    common::check_qpdf(&out_without);
    assert!(!contains(&bytes_without, b"/Info") && !contains(&bytes_without, b"/Producer"));
    assert_collapsed_outline_links(&bytes_without, Some(8));

    let reopened_without = PdfDocument::from_bytes(&bytes_without).expect("reopen without");
    assert_eq!(simplify(&reopened_without), expected_without);
}

#[test]
fn poc_1_existing_add_bookmark_cannot_express_merged_outline_alone() {
    let mut doc = PdfDocument::new_blank().unwrap();
    let prepared = prepare_for_merge("one", &corpus("benign/single-page.pdf"));
    doc.insert_pages(0, &prepared.bytes, &[]).unwrap();

    // Existing `add_bookmark`:
    // 1. Always creates an expanded bookmark (`closed: false`, positive /Count) with no collapse API.
    // 2. Requires a page (`u32`) and always writes `/Dest` (cannot create a title-only item).
    let parent = doc.add_bookmark(None, 0, "Parent", 0).unwrap();
    let _child = doc.add_bookmark(Some(parent), 0, "Child", 0).unwrap();
    let mut bytes = Vec::new();
    doc.write_merged(&mut bytes).unwrap();

    let raw_doc = MuPdfDocument::from_bytes(&bytes).unwrap();
    let first = raw_doc
        .catalog()
        .unwrap()
        .get_dict("Outlines")
        .unwrap()
        .unwrap()
        .get_dict("First")
        .unwrap()
        .unwrap();
    assert_eq!(
        first.get_dict("Count").unwrap().unwrap().as_int().unwrap(),
        1,
        "add_bookmark always writes positive (expanded) /Count"
    );
    assert!(first.get_dict("Dest").unwrap().is_some());
}

fn expand_qdf(pdf_bytes: &[u8], tag: &str) -> Vec<u8> {
    let qpdf = common::find_qpdf().expect("qpdf");
    let input = common::temp_pdf(&format!("{tag}-in"), pdf_bytes);
    let output = common::temp_pdf(&format!("{tag}-qdf"), b"");
    let status = Command::new(qpdf)
        .arg("--qdf")
        .arg("--object-streams=disable")
        .arg(&*input)
        .arg(&*output)
        .output()
        .expect("run qpdf --qdf");
    assert!(
        status.status.success(),
        "qpdf --qdf failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read(&*output).expect("read expanded qdf")
}

#[test]
fn poc_3_malicious_actions_and_next_chains_are_stripped() {
    let malicious_outline_pdf = build_pdf_with_outline(
        &["Malicious Outline Source Page 1"],
        &[
            OutlineSpec {
                title_pdf: pdf_string("JS Bookmark"),
                depth: 0,
                page: None,
                extra: "/A << /S /JavaScript /JS (app.alert\\(1\\)) >>",
            },
            OutlineSpec {
                title_pdf: pdf_string("Launch Bookmark"),
                depth: 0,
                page: None,
                extra: "/A << /S /Launch /F (calc.exe) >>",
            },
            OutlineSpec {
                title_pdf: pdf_string("GoToR Bookmark"),
                depth: 0,
                page: None,
                extra: "/A << /S /GoToR /F (secret.pdf) /D [0 /Fit] >>",
            },
            OutlineSpec {
                title_pdf: pdf_string("URI Bookmark"),
                depth: 0,
                page: None,
                extra: "/A << /S /URI /URI (https://example.invalid/evil) >>",
            },
            OutlineSpec {
                title_pdf: pdf_string("Chained URI Next JS"),
                depth: 0,
                page: None,
                extra: "/A << /S /URI /URI (https://example.invalid/chain) /Next << /S /JavaScript /JS (app.alert\\(2\\)) >> >> /C [1 0 0] /F 2",
            },
            OutlineSpec {
                title_pdf: pdf_string("Chained Launch Next GoToR"),
                depth: 0,
                page: None,
                extra: "/A << /S /Launch /F (evil.exe) /Next << /S /GoToR /F (remote.pdf) /D [0 /Fit] >> >>",
            },
            OutlineSpec {
                title_pdf: pdf_string("Chained GoTo Next JS"),
                depth: 0,
                page: None,
                extra: "/A << /S /GoTo /D [0 /Fit] /Next << /S /JavaScript /JS (app.alert\\(3\\)) >> >>",
            },
            OutlineSpec {
                title_pdf: utf16_hex("  \u{202E}  "),
                depth: 0,
                page: None,
                extra: "/A << /S /JavaScript /JS (app.alert\\(4\\)) >>",
            },
            OutlineSpec {
                title_pdf: pdf_string("Valid Jump"),
                depth: 0,
                page: Some(0),
                extra: "",
            },
        ],
        " /OpenAction << /S /JavaScript /JS (app.alert\\(0\\)) >>",
        " /AA << /O << /S /JavaScript /JS (app.alert\\(9\\)) >> >>",
    );

    let sources = vec![
        prepare_for_merge("actions-source", &malicious_outline_pdf),
        prepare_for_merge("outline-3-levels", &corpus("benign/outline-3-levels.pdf")),
    ];
    let (_merged, out_bytes) = merge_sources(&sources, true);
    let out_path = poc_dir().join("poc3-actions-stripped.pdf");
    std::fs::write(&out_path, &out_bytes).expect("write poc3");
    common::check_qpdf(&out_path);
    assert_collapsed_outline_links(&out_bytes, Some(2));

    let reopened = PdfDocument::from_bytes(&out_bytes).expect("reopen poc3");
    let scan = reopened.active_content(ScanBudget::default());
    assert!(scan.scan_complete);
    assert!(scan.findings.is_empty(), "{:?}", scan.findings);
    assert!(
        scan.remote_addresses.is_empty(),
        "{:?}",
        scan.remote_addresses
    );

    let outline = reopened
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap();
    let simplified: Vec<(&str, u16, &OutlineTarget)> = outline
        .entries
        .iter()
        .map(|e| (e.title.as_str(), e.depth, &e.target))
        .collect();
    assert_eq!(
        simplified[..10],
        [
            ("actions-source", 0, &OutlineTarget::Page(0)),
            ("JS Bookmark", 1, &OutlineTarget::None),
            ("Launch Bookmark", 1, &OutlineTarget::None),
            ("GoToR Bookmark", 1, &OutlineTarget::None),
            ("URI Bookmark", 1, &OutlineTarget::None),
            ("Chained URI Next JS", 1, &OutlineTarget::None),
            ("Chained Launch Next GoToR", 1, &OutlineTarget::None),
            // Note: `action_target` handles `/S /GoTo` without inspecting `/Next`, so it maps to Page(0),
            // while outline rebuilding drops `/A` and `/Next` completely and writes only explicit `/Dest`.
            ("Chained GoTo Next JS", 1, &OutlineTarget::Page(0)),
            ("（沒有標題）", 1, &OutlineTarget::None),
            ("Valid Jump", 1, &OutlineTarget::Page(0)),
        ]
    );

    // Expand with `qpdf --qdf` and verify none of the action tokens remain anywhere in the file.
    let qdf = expand_qdf(&out_bytes, "poc3-actions");
    for forbidden in [
        &b"/JS"[..],
        b"/JavaScript",
        b"/Launch",
        b"/GoToR",
        b"/URI",
        b"/OpenAction",
        b"/AA",
        b"/A ",
        b"/A\n",
        b"/A<",
    ] {
        assert!(
            !contains(&qdf, forbidden),
            "expanded QDF unexpectedly contains {}",
            String::from_utf8_lossy(forbidden)
        );
    }
}

fn nested_chain_pdf(count: usize) -> Vec<u8> {
    let entries: Vec<OutlineSpec<'_>> = (0..count)
        .map(|depth| OutlineSpec {
            title_pdf: pdf_string(&format!("Level {depth}")),
            depth,
            page: Some(0),
            extra: "",
        })
        .collect();
    build_pdf_with_outline(&["Nested outline page"], &entries, "", "")
}

fn flat_items_pdf(prefix: &str, count: usize) -> Vec<u8> {
    let entries: Vec<OutlineSpec<'_>> = (0..count)
        .map(|i| OutlineSpec {
            title_pdf: pdf_string(&format!("{prefix}-{i}")),
            depth: 0,
            page: Some(0),
            extra: "",
        })
        .collect();
    build_pdf_with_outline(&["Flat outline page"], &entries, "", "")
}

#[test]
fn poc_3_cycle_depth_and_item_limit_drop_whole_source_outline() {
    // 1. Cycle (`malformed/outline-cycle.pdf`): `outline()` sets `truncated = true`, so its own
    //    outline is dropped completely rather than keeping partial entries.
    let cycle = prepare_for_merge("outline-cycle", &corpus("malformed/outline-cycle.pdf"));
    assert!(cycle.outline.truncated);
    let three_levels =
        prepare_for_merge("outline-3-levels", &corpus("benign/outline-3-levels.pdf"));

    let (with_cycle, with_cycle_bytes) = merge_sources(&[cycle, three_levels], true);
    common::check_qpdf_bytes(&with_cycle_bytes);
    let outline_with = with_cycle
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap();
    assert_eq!(outline_with.entries.len(), 1 + 1 + 6);
    assert_eq!(outline_with.entries[0].title, "outline-cycle");
    assert_eq!(outline_with.entries[1].title, "outline-3-levels");

    let cycle = prepare_for_merge("outline-cycle", &corpus("malformed/outline-cycle.pdf"));
    let three_levels =
        prepare_for_merge("outline-3-levels", &corpus("benign/outline-3-levels.pdf"));
    let (without_cycle, without_cycle_bytes) = merge_sources(&[cycle, three_levels], false);
    common::check_qpdf_bytes(&without_cycle_bytes);
    let outline_without = without_cycle
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap();
    assert_eq!(outline_without.entries.len(), 6);
    assert_eq!(outline_without.entries[0].title, "Chapter 1");

    // 2. Depth limits:
    //    `depth` is 0-based in `engine.rs` / `outline.rs` / `validate.rs`, so `MAX_OUTLINE_DEPTH`
    //    (64) allows depths 0..=64 (65 levels). Under a per-source bookmark (`depth = 0`), a source
    //    may reach depth 0..=63 (64 levels -> merged depths 1..=64). A 65-level source (depths
    //    0..=64) is NOT truncated on read, yet exceeds 64 when wrapped and must be dropped whole.
    let chain_64 = prepare_for_merge("chain-64-levels", &nested_chain_pdf(64));
    assert!(!chain_64.outline.truncated);
    assert_eq!(chain_64.outline.entries.last().unwrap().depth, 63);

    let chain_65 = prepare_for_merge("chain-65-levels", &nested_chain_pdf(65));
    assert!(!chain_65.outline.truncated);
    assert_eq!(chain_65.outline.entries.last().unwrap().depth, 64);

    let chain_66 = prepare_for_merge("chain-66-levels", &nested_chain_pdf(66));
    assert!(chain_66.outline.truncated);

    // With per-source bookmark: 64-level (max depth 63) fits; 65-level (max depth 64) & 66-level drop.
    let (depth_with, depth_with_bytes) = merge_sources(&[chain_64, chain_65, chain_66], true);
    common::check_qpdf_bytes(&depth_with_bytes);
    let outline_depth_with = depth_with
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap();
    assert!(!outline_depth_with.truncated);
    assert_eq!(outline_depth_with.entries.len(), 1 + 64 + 1 + 1);
    assert_eq!(outline_depth_with.entries[64].depth, 64);
    assert_eq!(outline_depth_with.entries[65].title, "chain-65-levels");
    assert_eq!(outline_depth_with.entries[66].title, "chain-66-levels");

    // Without per-source bookmark: 65-level (max depth 64) fits; 66-level drops.
    let chain_65_b = prepare_for_merge("chain-65-levels", &nested_chain_pdf(65));
    let chain_66_b = prepare_for_merge("chain-66-levels", &nested_chain_pdf(66));
    let (depth_without, _) = merge_sources(&[chain_65_b, chain_66_b], false);
    let outline_depth_without = depth_without
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap();
    assert!(!outline_depth_without.truncated);
    assert_eq!(outline_depth_without.entries.len(), 65);
    assert_eq!(outline_depth_without.entries.last().unwrap().depth, 64);

    // 3. Total item limit (10,000) and order sensitivity:
    //    S1 = 9,989 items (+1 source = 9,990), S2 = 7 items, S3 = 6 items.
    //    Remaining after S1 is 10 items.
    //    In order [S1, S2, S3]: S2 needs 1+7 (+1 reserved for S3 = 9 <= 10) -> S2 fits, S3 drops.
    //    In order [S1, S3, S2]: S3 needs 1+6 (+1 reserved for S2 = 8 <= 10) -> S3 fits, S2 drops.
    let s1 = prepare_for_merge("s1", &flat_items_pdf("s1", 9_989));
    let s2 = prepare_for_merge("s2", &flat_items_pdf("s2", 7));
    let s3 = prepare_for_merge("s3", &flat_items_pdf("s3", 6));

    let (order_a, bytes_a) = merge_sources(&[s1, s2, s3], true);
    common::check_qpdf_bytes(&bytes_a);
    let out_a = order_a
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap();
    assert!(!out_a.truncated);
    assert_eq!(out_a.entries.len(), 1 + 9_989 + 1 + 7 + 1); // 9,999
    assert!(out_a.entries.iter().any(|e| e.title == "s2-6"));
    assert!(!out_a.entries.iter().any(|e| e.title == "s3-0"));

    let s1 = prepare_for_merge("s1", &flat_items_pdf("s1", 9_989));
    let s2 = prepare_for_merge("s2", &flat_items_pdf("s2", 7));
    let s3 = prepare_for_merge("s3", &flat_items_pdf("s3", 6));

    let (order_b, bytes_b) = merge_sources(&[s1, s3, s2], true);
    common::check_qpdf_bytes(&bytes_b);
    let out_b = order_b
        .outline(MAX_OUTLINE_ITEMS as usize, MAX_OUTLINE_DEPTH)
        .unwrap();
    assert!(!out_b.truncated);
    assert_eq!(out_b.entries.len(), 1 + 9_989 + 1 + 6 + 1); // 9,998
    assert!(out_b.entries.iter().any(|e| e.title == "s3-5"));
    assert!(!out_b.entries.iter().any(|e| e.title == "s2-0"));
}

fn count_nearly_red(rgba: &[u8]) -> usize {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] >= 240 && px[1] <= 15 && px[2] <= 15 && px[3] >= 240)
        .count()
}

fn count_nearly_blue(rgba: &[u8]) -> usize {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[0] <= 15 && px[1] <= 15 && px[2] >= 240 && px[3] >= 240)
        .count()
}

#[test]
fn poc_4_layers_detection_and_flattening_to_visible() {
    let hidden_samples = [
        ("layers-hidden", "benign/layers-hidden.pdf", true),
        ("layers-base-off", "benign/layers-base-off.pdf", true),
        (
            "layers-intent-design",
            "benign/layers-intent-design.pdf",
            true,
        ),
        // MuPDF 1.28.6 renders OCMD /VE as visible on its own; Acrobat/Edge/Chrome/Firefox hide it.
        ("layers-ocmd-ve", "benign/layers-ocmd-ve.pdf", false),
    ];

    let mut merge_inputs = Vec::new();
    for (stem, path, expect_hidden_in_mupdf) in hidden_samples {
        let bytes = corpus(path);
        let own = PdfDocument::from_bytes(&bytes).expect("open source");
        let own_red = count_nearly_red(&own.render(0, 1.0, 0).expect("render own").rgba);
        if expect_hidden_in_mupdf {
            assert_eq!(own_red, 0, "{path} should hide red square on its own");
        } else {
            assert!(own_red > 10_000, "{path} own_red={own_red}");
        }

        let prepared = prepare_for_merge(stem, &bytes);
        assert!(prepared.has_layers, "{path} must report has_layers=true");
        merge_inputs.push(prepared);
    }

    let visible = prepare_for_merge("layers-visible", &corpus("benign/layers-visible.pdf"));
    assert!(visible.has_layers, "layers-visible.pdf has_layers=true");
    merge_inputs.push(visible);

    // Negative controls: no /OCProperties and empty /OCGs [].
    let no_props = prepare_for_merge("single-page", &corpus("benign/single-page.pdf"));
    assert!(!no_props.has_layers);
    let empty_ocgs =
        prepare_for_merge("layers-empty-ocgs", &corpus("benign/layers-empty-ocgs.pdf"));
    assert!(!empty_ocgs.has_layers);

    let (_merged, merged_bytes) = merge_sources(&merge_inputs, true);
    let out_pdf = poc_dir().join("poc4-layers-merged.pdf");
    std::fs::write(&out_pdf, &merged_bytes).expect("write poc4 pdf");
    common::check_qpdf(&out_pdf);

    let qdf = expand_qdf(&merged_bytes, "poc4-layers");
    assert!(!contains(&qdf, b"/OCProperties"));

    let reopened = PdfDocument::from_bytes(&merged_bytes).expect("reopen poc4");
    assert!(!reopened.has_layers());
    assert_eq!(reopened.page_count().unwrap(), 5);

    let page_names = [
        "poc4-page0-layers-hidden.png",
        "poc4-page1-layers-base-off.png",
        "poc4-page2-layers-intent-design.png",
        "poc4-page3-layers-ocmd-ve.png",
        "poc4-page4-layers-visible.png",
    ];

    for (page, png_name) in page_names.iter().enumerate() {
        let page_idx = page as u32;
        let rendered = reopened.render(page_idx, 1.0, 0).expect("render page");
        if page < 4 {
            let red = count_nearly_red(&rendered.rgba);
            assert!(
                red > 10_000,
                "merged page {page} ({png_name}) must show hidden red layer, got {red} pixels"
            );
        } else {
            let blue = count_nearly_blue(&rendered.rgba);
            assert!(
                blue > 10_000,
                "merged page {page} ({png_name}) must show blue visible layer, got {blue} pixels"
            );
        }
        let png = reopened.render_png(page_idx, 1.0).expect("render png");
        std::fs::write(poc_dir().join(png_name), &png).expect("write png");
    }
}
