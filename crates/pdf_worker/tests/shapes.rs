//! Integration tests for Polygon, PolyLine, and cloudy border annotations (B7-10, Issue #745).
#![cfg(windows)]

mod common;

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

use ipc_contract::limits::{MAX_PAGE_SIDE_PT, MAX_SHAPE_VERTICES};
use ipc_contract::types::{
    AnnotationKind, AnnotationSummaryKind, DocumentId, ErrorCode, InkColor, InkWidth, PageSize,
    Point, Rect, Rotation, Shape,
};
use ipc_contract::worker::{ExportedAnnotationKind, WorkerEdit, WorkerRequest, WorkerResponse};
use worker_host::{HostConfig, WorkerHost};

fn host() -> WorkerHost {
    WorkerHost::new(
        Path::new(env!("CARGO_BIN_EXE_pdf_worker")),
        HostConfig::default(),
    )
}

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus")
        .join(name)
}

fn opened(host: &mut WorkerHost, path: &Path) -> (DocumentId, Vec<PageSize>) {
    match host.open(path).expect("open") {
        (doc, WorkerResponse::Opened { document, .. }) => (doc, document.pages),
        other => panic!("{other:?}"),
    }
}

fn blank_pdf() -> Vec<u8> {
    common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>".to_owned(),
    ])
}

fn new_file(name: &str) -> (common::TempFileGuard, File) {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "pdf-worker-{}-shapes-{id}-{name}.pdf",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .expect("create");
    (common::TempFileGuard(path), file)
}

fn page_annots(
    host: &mut WorkerHost,
    doc: DocumentId,
    page_index: u32,
) -> Vec<ipc_contract::types::PageAnnotation> {
    match host
        .request(|request| WorkerRequest::GetPageAnnotations {
            request,
            doc,
            page_index,
        })
        .expect("page annotations")
    {
        WorkerResponse::PageAnnotations { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    }
}

fn render_rgba(host: &mut WorkerHost, doc: DocumentId, page_index: u32) -> Vec<u8> {
    match host
        .request(|request| WorkerRequest::Render {
            request,
            doc,
            page_index,
            scale: 2.0,
            rotation: Rotation::None,
        })
        .expect("render")
    {
        WorkerResponse::Rendered { raster, .. } => raster.pixels,
        other => panic!("{other:?}"),
    }
}

fn diff_pixels(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0.iter())
        .filter(|(p, q)| p != q)
        .count()
}

#[test]
fn app_cloudy_square_border_is_recognized_exported_and_merged() {
    let pdf = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Annots [4 0 R] >>".to_owned(),
        "<< /Type /Annot /Subtype /Square /Rect [50 642 150 742] /C [0.85 0.1 0.1] /BS << /W 2.5 >> /BE << /S /C /I 1 >> >>".to_owned(),
    ]);
    let src_file = common::temp_pdf("cloudy-square-src", &pdf);
    let mut h = host();
    let (doc, _) = opened(&mut h, &src_file);

    let annots = page_annots(&mut h, doc, 0);
    assert_eq!(annots.len(), 1);
    assert_eq!(annots[0].kind, AnnotationKind::Rectangle);
    assert!(annots[0].shape_cloudy);

    let exported = match h
        .request(|request| WorkerRequest::AnnotationExport { request, doc })
        .expect("annotation export")
    {
        WorkerResponse::AnnotationExport { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };
    assert_eq!(exported.items.len(), 1);
    assert_eq!(exported.items[0].kind, ExportedAnnotationKind::Rectangle);
    assert!(exported.items[0].cloudy);

    let target_file = common::temp_pdf("cloudy-square-target", &blank_pdf());
    let (target_doc, _) = opened(&mut h, &target_file);
    let f = File::open(&src_file).expect("open src");
    let source_bytes = match h.prepare_source(&f, None).expect("prepare_source") {
        WorkerResponse::Source { bytes, .. } => bytes,
        other => panic!("{other:?}"),
    };
    h.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 0,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("insert");
    let merged_annots = page_annots(&mut h, target_doc, 0);
    assert_eq!(merged_annots.len(), 1);
    assert_eq!(merged_annots[0].kind, AnnotationKind::Rectangle);
    assert!(merged_annots[0].shape_cloudy);
    let (saved_guard, saved_file) = new_file("cloudy-square-merged");
    h.save(target_doc, &saved_file).expect("save");
    drop(saved_file);
    common::check_qpdf(&saved_guard);
}

#[test]
fn polygon_polyline_and_cloudy_borders_create_render_edit_save_merge_and_export() {
    let src_file = common::temp_pdf("shapes-full", &blank_pdf());
    let mut h = host();
    let (doc, _) = opened(&mut h, &src_file);
    let blank_px = render_rgba(&mut h, doc, 0);

    // 1. Add Cloudy Rectangle
    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::AddShape {
            page: 0,
            shape: Shape::Rectangle {
                rect: Rect {
                    x0: 40.0,
                    y0: 40.0,
                    x1: 140.0,
                    y1: 120.0,
                },
                cloudy: true,
            },
            color: InkColor::Red,
            width: InkWidth::Thin,
        },
    })
    .expect("add cloudy rectangle");

    // 2. Add Cloudy Polygon
    let poly_pts = vec![
        Point { x: 200.0, y: 50.0 },
        Point { x: 320.0, y: 70.0 },
        Point { x: 260.0, y: 170.0 },
    ];
    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::AddShape {
            page: 0,
            shape: Shape::Polygon {
                vertices: poly_pts.clone(),
                cloudy: true,
            },
            color: InkColor::Blue,
            width: InkWidth::Medium,
        },
    })
    .expect("add cloudy polygon");

    // 3. Add PolyLine
    let polyline_pts = vec![
        Point { x: 60.0, y: 220.0 },
        Point { x: 180.0, y: 220.0 },
        Point { x: 240.0, y: 290.0 },
    ];
    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::AddShape {
            page: 0,
            shape: Shape::PolyLine {
                vertices: polyline_pts.clone(),
            },
            color: InkColor::Green,
            width: InkWidth::Thick,
        },
    })
    .expect("add polyline");

    // Rendered pixels must differ visibly from blank page
    let drawn_px = render_rgba(&mut h, doc, 0);
    assert!(
        diff_pixels(&drawn_px, &blank_px) > 1000,
        "shapes must render visible pixels"
    );

    // Toggle cloudy off vs on comparison: rendered pixels differ!
    let annots = page_annots(&mut h, doc, 0);
    assert_eq!(annots.len(), 3);
    let rect_id = annots
        .iter()
        .find(|a| a.kind == AnnotationKind::Rectangle)
        .unwrap()
        .id;
    let poly_id = annots
        .iter()
        .find(|a| a.kind == AnnotationKind::Polygon)
        .unwrap()
        .id;
    let polyline_id = annots
        .iter()
        .find(|a| a.kind == AnnotationKind::PolyLine)
        .unwrap()
        .id;

    // Move Polygon vertex via SetShapeVertices
    let moved_vertex = vec![
        Point { x: 210.0, y: 60.0 },
        Point { x: 330.0, y: 80.0 },
        Point { x: 260.0, y: 190.0 },
    ];
    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::SetShapeVertices {
            page: 0,
            annotation: poly_id,
            vertices: moved_vertex.clone(),
        },
    })
    .expect("move vertex");

    // Move entire PolyLine via SetAnnotationRect (translate vertices by (+15, +25))
    let old_poly_rect = page_annots(&mut h, doc, 0)
        .into_iter()
        .find(|a| a.id == polyline_id)
        .unwrap()
        .rect;
    let shifted_rect = Rect {
        x0: old_poly_rect.x0 + 15.0,
        y0: old_poly_rect.y0 + 25.0,
        x1: (old_poly_rect.x1 + 15.0).max(old_poly_rect.x0 + 15.0 + 10.0),
        y1: (old_poly_rect.y1 + 25.0).max(old_poly_rect.y0 + 25.0 + 10.0),
    };
    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::SetAnnotationRect {
            page: 0,
            annotation: polyline_id,
            rect: shifted_rect,
        },
    })
    .expect("move polyline");

    // Restyle PolyLine color & width
    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::SetShapeStyle {
            page: 0,
            annotation: polyline_id,
            color: InkColor::Red,
            width: InkWidth::Thin,
        },
    })
    .expect("restyle polyline");

    // Toggle Rectangle cloudy off -> pixels differ from cloudy; then back on
    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::SetShapeCloudy {
            page: 0,
            annotation: rect_id,
            cloudy: false,
        },
    })
    .expect("cloudy off");
    let after_plain_rect_px = render_rgba(&mut h, doc, 0);
    assert!(diff_pixels(&after_plain_rect_px, &drawn_px) > 100);
    assert!(
        !page_annots(&mut h, doc, 0)
            .iter()
            .find(|a| a.id == rect_id)
            .unwrap()
            .shape_cloudy
    );

    h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::SetShapeCloudy {
            page: 0,
            annotation: rect_id,
            cloudy: true,
        },
    })
    .expect("cloudy back on");

    // Reject SetShapeCloudy on PolyLine
    let bad_cloudy = h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::SetShapeCloudy {
            page: 0,
            annotation: polyline_id,
            cloudy: true,
        },
    });
    assert!(bad_cloudy.is_err());

    // Reject SetShapeVertices on Polygon with < 3 vertices
    let too_few = h.request(|request| WorkerRequest::Edit {
        request,
        doc,
        edit: WorkerEdit::SetShapeVertices {
            page: 0,
            annotation: poly_id,
            vertices: vec![Point { x: 10.0, y: 10.0 }, Point { x: 20.0, y: 20.0 }],
        },
    });
    assert!(too_few.is_err());

    // Save & reopen & check qpdf
    let (saved_guard, saved_file) = new_file("shapes-full-saved");
    h.save(doc, &saved_file).expect("save");
    drop(saved_file);
    common::check_qpdf(&saved_guard);

    let (reopened_doc, _) = opened(&mut h, &saved_guard);
    let reopened = page_annots(&mut h, reopened_doc, 0);
    assert_eq!(reopened.len(), 3);

    let r_rect = reopened
        .iter()
        .find(|a| a.kind == AnnotationKind::Rectangle)
        .unwrap();
    assert!(r_rect.shape_cloudy);
    assert_eq!(r_rect.shape_color, Some(InkColor::Red));
    assert_eq!(r_rect.shape_width, Some(InkWidth::Thin));

    let r_poly = reopened
        .iter()
        .find(|a| a.kind == AnnotationKind::Polygon)
        .unwrap();
    assert!(r_poly.shape_cloudy);
    assert_eq!(r_poly.shape_color, Some(InkColor::Blue));
    assert_eq!(r_poly.shape_width, Some(InkWidth::Medium));
    let verts = r_poly.vertices.as_ref().expect("polygon vertices");
    assert_eq!(verts.len(), 3);
    for (got, expected) in verts.iter().zip(moved_vertex.iter()) {
        assert!((got.x - expected.x).abs() < 0.1 && (got.y - expected.y).abs() < 0.1);
    }

    let r_pl = reopened
        .iter()
        .find(|a| a.kind == AnnotationKind::PolyLine)
        .unwrap();
    assert!(!r_pl.shape_cloudy);
    assert_eq!(r_pl.shape_color, Some(InkColor::Red));
    assert_eq!(r_pl.shape_width, Some(InkWidth::Thin));
    let pl_verts = r_pl.vertices.as_ref().expect("polyline vertices");
    for (got, orig) in pl_verts.iter().zip(polyline_pts.iter()) {
        assert!((got.x - (orig.x + 15.0)).abs() < 0.1);
        assert!((got.y - (orig.y + 25.0)).abs() < 0.1);
    }

    // Annotation summary lists Polygon, PolyLine, and Rectangle in reading order
    let summary = match h
        .request(|request| WorkerRequest::AnnotationSummary {
            request,
            doc: reopened_doc,
        })
        .expect("summary")
    {
        WorkerResponse::AnnotationSummary { summary, .. } => summary,
        other => panic!("{other:?}"),
    };
    let kinds: Vec<_> = summary.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        vec![
            AnnotationSummaryKind::Rectangle,
            AnnotationSummaryKind::Polygon,
            AnnotationSummaryKind::PolyLine,
        ]
    );

    // Export XFDF contains vertices & cloudy
    let xfdf = match h
        .request(|request| WorkerRequest::AnnotationExport {
            request,
            doc: reopened_doc,
        })
        .expect("xfdf")
    {
        WorkerResponse::AnnotationExport { annotations, .. } => annotations,
        other => panic!("{other:?}"),
    };
    let x_poly = xfdf
        .items
        .iter()
        .find(|i| i.kind == ExportedAnnotationKind::Polygon)
        .unwrap();
    assert!(x_poly.cloudy);
    assert_eq!(x_poly.vertices.len(), 3);
    let x_pl = xfdf
        .items
        .iter()
        .find(|i| i.kind == ExportedAnnotationKind::PolyLine)
        .unwrap();
    assert!(!x_pl.cloudy);
    assert_eq!(x_pl.vertices.len(), 3);

    // Merge into another document preserves all 3
    let target_file = common::temp_pdf("shapes-merge-target", &blank_pdf());
    let (target_doc, _) = opened(&mut h, &target_file);
    let f = File::open(&saved_guard).expect("open saved");
    let source_bytes = match h.prepare_source(&f, None).expect("prepare source") {
        WorkerResponse::Source { bytes, .. } => bytes,
        other => panic!("{other:?}"),
    };
    h.request(|request| WorkerRequest::Edit {
        request,
        doc: target_doc,
        edit: WorkerEdit::InsertPages {
            at: 0,
            source: source_bytes,
            text_boxes: Vec::new(),
        },
    })
    .expect("merge insert");
    let merged = page_annots(&mut h, target_doc, 0);
    assert_eq!(merged.len(), 3);
    assert!(
        merged
            .iter()
            .any(|a| a.kind == AnnotationKind::Polygon && a.shape_cloudy)
    );
    assert!(merged.iter().any(|a| a.kind == AnnotationKind::PolyLine));
    assert!(
        merged
            .iter()
            .any(|a| a.kind == AnnotationKind::Rectangle && a.shape_cloudy)
    );
}

#[test]
fn worker_rejects_invalid_and_out_of_page_vertices_and_downgrades_foreign_polygons() {
    // Foreign PDF with Polygon carrying /IT /PolygonCloud, Polygon carrying /Measure, PolyLine carrying /LE
    let foreign = common::build_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [4 0 R 5 0 R 6 0 R] >>".to_owned(),
        "<< /Type /Annot /Subtype /Polygon /Rect [50 600 150 700] /Vertices [50 700 150 700 100 600] /IT /PolygonCloud /BE << /S /C /I 1 >> /C [1 0 0] >>".to_owned(),
        "<< /Type /Annot /Subtype /Polygon /Rect [200 600 300 700] /Vertices [200 700 300 700 250 600] /Measure << /Type /Measure >> /C [0 0 1] >>".to_owned(),
        "<< /Type /Annot /Subtype /PolyLine /Rect [50 400 150 500] /Vertices [50 400 150 500] /LE [/None /OpenArrow] /C [0 1 0] >>".to_owned(),
    ]);
    let src_file = common::temp_pdf("foreign-polygons", &foreign);
    let mut h = host();
    let (doc, _) = opened(&mut h, &src_file);

    let annots = page_annots(&mut h, doc, 0);
    assert_eq!(annots.len(), 3);
    for annot in &annots {
        assert_eq!(
            annot.kind,
            AnnotationKind::Other,
            "foreign shape must stay Other: {annot:?}"
        );
        assert!(
            h.request(|request| WorkerRequest::Edit {
                request,
                doc,
                edit: WorkerEdit::SetShapeStyle {
                    page: 0,
                    annotation: annot.id,
                    color: InkColor::Red,
                    width: InkWidth::Thin,
                },
            })
            .is_err()
        );
    }

    // Worker rejects NaN, 1e39 (infinity), too many, too few, adjacent identical, and off-page vertices
    let bad_shapes = vec![
        Shape::Polygon {
            vertices: vec![
                Point {
                    x: f32::NAN,
                    y: 10.0,
                },
                Point { x: 50.0, y: 10.0 },
                Point { x: 30.0, y: 60.0 },
            ],
            cloudy: false,
        },
        Shape::Polygon {
            vertices: vec![
                Point {
                    x: (1e39_f64 as f32),
                    y: 10.0,
                },
                Point { x: 50.0, y: 10.0 },
                Point { x: 30.0, y: 60.0 },
            ],
            cloudy: false,
        },
        Shape::Polygon {
            vertices: vec![Point { x: 10.0, y: 10.0 }, Point { x: 50.0, y: 10.0 }],
            cloudy: false,
        },
        Shape::PolyLine {
            vertices: vec![Point { x: 10.0, y: 10.0 }],
        },
        Shape::PolyLine {
            vertices: vec![Point { x: 10.0, y: 10.0 }, Point { x: 10.0, y: 10.0 }],
        },
        Shape::PolyLine {
            vertices: (0..=MAX_SHAPE_VERTICES)
                .map(|i| Point {
                    x: (i % 500) as f32 + 10.0,
                    y: (i / 500) as f32 * 20.0 + 10.0,
                })
                .collect(),
        },
        // Out of 612x792 page bounds: negative and > width_pt
        Shape::Polygon {
            vertices: vec![
                Point { x: -5.0, y: 10.0 },
                Point { x: 50.0, y: 10.0 },
                Point { x: 30.0, y: 60.0 },
            ],
            cloudy: false,
        },
        Shape::Polygon {
            vertices: vec![
                Point { x: 10.0, y: 10.0 },
                Point { x: 700.0, y: 10.0 },
                Point { x: 30.0, y: 60.0 },
            ],
            cloudy: false,
        },
        Shape::PolyLine {
            vertices: vec![
                Point { x: 10.0, y: 10.0 },
                Point {
                    x: 10.0,
                    y: MAX_PAGE_SIDE_PT,
                },
            ],
        },
    ];

    for bad in bad_shapes {
        let err = h
            .request(|request| WorkerRequest::Edit {
                request,
                doc,
                edit: WorkerEdit::AddShape {
                    page: 0,
                    shape: bad.clone(),
                    color: InkColor::Black,
                    width: InkWidth::Thin,
                },
            })
            .unwrap_err();
        assert_eq!(
            err.code(),
            ErrorCode::InvalidArgument,
            "expected InvalidArgument for {bad:?}"
        );
    }
}

#[test]
fn polygon_and_polyline_round_trip_on_rotated_pages() {
    let mut h = host();
    let (doc, pages) = opened(&mut h, &corpus("benign/rotated-page.pdf"));
    assert!(pages.len() >= 2);
    for page_idx in 0..2u32 {
        let verts = vec![
            Point { x: 50.0, y: 60.0 },
            Point { x: 150.0, y: 60.0 },
            Point { x: 100.0, y: 140.0 },
        ];
        h.request(|request| WorkerRequest::Edit {
            request,
            doc,
            edit: WorkerEdit::AddShape {
                page: page_idx,
                shape: Shape::Polygon {
                    vertices: verts.clone(),
                    cloudy: true,
                },
                color: InkColor::Blue,
                width: InkWidth::Medium,
            },
        })
        .expect("add polygon on rotated page");
    }

    let (saved_guard, saved_file) = new_file("rotated-polygons");
    h.save(doc, &saved_file).expect("save");
    drop(saved_file);
    common::check_qpdf(&saved_guard);

    let (reopened_doc, _) = opened(&mut h, &saved_guard);
    for page_idx in 0..2u32 {
        let annots = page_annots(&mut h, reopened_doc, page_idx);
        let poly = annots
            .iter()
            .find(|a| a.kind == AnnotationKind::Polygon)
            .unwrap();
        assert!(poly.shape_cloudy);
        let got = poly.vertices.as_ref().unwrap();
        assert_eq!(got.len(), 3);
        assert!((got[0].x - 50.0).abs() < 0.1 && (got[0].y - 60.0).abs() < 0.1);
        assert!((got[2].x - 100.0).abs() < 0.1 && (got[2].y - 140.0).abs() < 0.1);
    }
}
