//! Safe wrapper around MuPDF for opening PDFs and rendering pages (MVP-03).
//!
//! MuPDF is built without its JavaScript engine (`FZ_ENABLE_JS=0`, see
//! docs/architecture/mupdf-binding.md), so no PDF script can run even if something asked for it.
//! Nothing in this crate may call `PdfDocument::enable_js`.

use mupdf::{Colorspace, Document, Matrix, Page};
use thiserror::Error;

/// PDF files must start with this header within the first 1024 bytes (PDF 1.7, 7.5.2).
const PDF_HEADER: &[u8] = b"%PDF-";
const HEADER_SEARCH_BYTES: usize = 1024;

/// Render scale bounds (mirrors `ipc_contract::limits`).
pub const MIN_RENDER_SCALE: f32 = 0.01;
pub const MAX_RENDER_SCALE: f32 = 64.0;

/// Largest raster the engine produces, checked before MuPDF allocates it
/// (mirrors `ipc_contract::limits::MAX_RASTER_PIXELS`).
pub const MAX_RASTER_PIXELS: u64 = 4096 * 4096;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("not a PDF document")]
    NotPdf,
    #[error("the document is encrypted")]
    Encrypted,
    #[error("page {0} does not exist")]
    PageOutOfRange(u32),
    #[error("render scale must be between {MIN_RENDER_SCALE} and {MAX_RENDER_SCALE}")]
    InvalidScale,
    #[error("rotation must be 0, 90, 180 or 270 degrees")]
    InvalidRotation,
    #[error("a {width} x {height} render exceeds the raster limit")]
    TooLarge { width: u64, height: u64 },
    #[error("MuPDF: {0}")]
    MuPdf(#[from] mupdf::Error),
}

/// A rendered page: opaque RGBA8, rows top to bottom, no padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// An open PDF document.
pub struct PdfDocument {
    doc: Document,
}

impl PdfDocument {
    /// Opens a PDF from memory. Encrypted documents are refused (not supported in the MVP).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EngineError> {
        let head = &bytes[..bytes.len().min(HEADER_SEARCH_BYTES)];
        if !head
            .windows(PDF_HEADER.len())
            .any(|window| window == PDF_HEADER)
        {
            return Err(EngineError::NotPdf);
        }
        let doc = Document::from_bytes(bytes, "application/pdf")?;
        if doc.needs_password()? {
            return Err(EngineError::Encrypted);
        }
        Ok(Self { doc })
    }

    pub fn page_count(&self) -> Result<u32, EngineError> {
        Ok(u32::try_from(self.doc.page_count()?).unwrap_or(0))
    }

    /// Page size in PDF points, before any view rotation.
    pub fn page_size(&self, index: u32) -> Result<(f32, f32), EngineError> {
        let bounds = self.load_page(index)?.bounds()?;
        Ok((bounds.x1 - bounds.x0, bounds.y1 - bounds.y0))
    }

    /// Renders a page at `scale` (1.0 = 72 dpi) rotated clockwise by `rotation` degrees.
    pub fn render(
        &self,
        index: u32,
        scale: f32,
        rotation: u16,
    ) -> Result<RenderedPage, EngineError> {
        if !scale.is_finite() || !(MIN_RENDER_SCALE..=MAX_RENDER_SCALE).contains(&scale) {
            return Err(EngineError::InvalidScale);
        }
        if !matches!(rotation, 0 | 90 | 180 | 270) {
            return Err(EngineError::InvalidRotation);
        }
        let page = self.load_page(index)?;

        // Reject oversized renders before MuPDF allocates the pixmap.
        let bounds = page.bounds()?;
        let to_px = |points: f32| (f64::from(points) * f64::from(scale)).ceil().max(1.0) as u64;
        let (mut width, mut height) = (to_px(bounds.x1 - bounds.x0), to_px(bounds.y1 - bounds.y0));
        if rotation % 180 == 90 {
            std::mem::swap(&mut width, &mut height);
        }
        if width.saturating_mul(height) > MAX_RASTER_PIXELS {
            return Err(EngineError::TooLarge { width, height });
        }

        let mut ctm = Matrix::new_scale(scale, scale);
        ctm.concat(Matrix::new_rotate(f32::from(rotation)));
        // No alpha: MuPDF paints an opaque white background, which the contract requires.
        let pixmap = page.to_pixmap(&ctm, &Colorspace::device_rgb(), false, false)?;

        let (width, height) = (pixmap.width(), pixmap.height());
        let channels = usize::from(pixmap.n());
        let stride = usize::try_from(pixmap.stride()).unwrap_or(0);
        let row_bytes = width as usize * channels;
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in pixmap.samples().chunks_exact(stride.max(1)) {
            for pixel in row[..row_bytes].chunks_exact(channels) {
                rgba.extend_from_slice(&pixel[..3]);
                rgba.push(255);
            }
        }
        Ok(RenderedPage {
            width,
            height,
            rgba,
        })
    }

    fn load_page(&self, index: u32) -> Result<Page, EngineError> {
        if index >= self.page_count()? {
            return Err(EngineError::PageOutOfRange(index));
        }
        Ok(self.doc.load_page(index as i32)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a PDF with a correct xref table from object bodies (object n = body n-1).
    fn build_pdf(objects: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn stream(content: &str) -> String {
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
    }

    /// Two Letter pages. Page 1: black square at (100,100)-(300,300) and Helvetica text near the top.
    fn two_page_pdf() -> Vec<u8> {
        build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 7 0 R >> >> >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 6 0 R >>",
            &stream("0 0 0 rg 100 100 200 200 re f\nBT /F1 36 Tf 72 700 Td (Hello MuPDF) Tj ET"),
            &stream(""),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        ])
    }

    fn pixel(page: &RenderedPage, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * page.width + x) * 4) as usize;
        page.rgba[at..at + 4].try_into().unwrap()
    }

    fn dark_pixels(page: &RenderedPage, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|&(x, y)| pixel(page, x, y)[0] < 128)
            .count()
    }

    #[test]
    fn reports_pages_and_sizes() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        assert_eq!(doc.page_count().unwrap(), 2);
        assert_eq!(doc.page_size(0).unwrap(), (612.0, 792.0));
    }

    #[test]
    fn renders_opaque_rgba_with_graphics_and_text() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let page = doc.render(0, 1.0, 0).unwrap();

        assert_eq!((page.width, page.height), (612, 792));
        assert_eq!(page.rgba.len(), 612 * 792 * 4);
        let (pixels, rest) = page.rgba.as_chunks::<4>();
        assert!(rest.is_empty());
        assert!(pixels.iter().all(|px| px[3] == 255), "must be opaque");
        // Square: PDF y=200 is 792-200=592 from the top.
        assert_eq!(pixel(&page, 200, 592), [0, 0, 0, 255]);
        assert_eq!(pixel(&page, 10, 10), [255, 255, 255, 255]);
        // Text drawn with the bundled base-14 Helvetica (baseline at y=700 -> 92 from top).
        assert!(
            dark_pixels(&page, 72, 60, 320, 95) > 200,
            "Helvetica text must render"
        );
        // The blank page is entirely white.
        let blank = doc.render(1, 0.5, 0).unwrap();
        assert!(
            blank
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|px| *px == [255; 4])
        );
    }

    #[test]
    fn scales_and_rotates() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        let scaled = doc.render(0, 2.0, 0).unwrap();
        assert_eq!((scaled.width, scaled.height), (1224, 1584));

        let rotated = doc.render(0, 1.0, 90).unwrap();
        assert_eq!((rotated.width, rotated.height), (792, 612));
        // Clockwise quarter turn of a W x H image maps (x, y) to (H - y, x): the square's centre
        // (200, 592) lands on (200, 200); a counter-clockwise turn would put it at (592, 412).
        assert_eq!(pixel(&rotated, 200, 200), [0, 0, 0, 255]);
        assert_eq!(pixel(&rotated, 592, 412), [255, 255, 255, 255]);
    }

    #[test]
    fn rejects_bad_render_requests_before_rendering() {
        let doc = PdfDocument::from_bytes(&two_page_pdf()).unwrap();
        assert!(matches!(
            doc.render(2, 1.0, 0),
            Err(EngineError::PageOutOfRange(2))
        ));
        assert!(matches!(
            doc.render(0, 0.0, 0),
            Err(EngineError::InvalidScale)
        ));
        assert!(matches!(
            doc.render(0, f32::NAN, 0),
            Err(EngineError::InvalidScale)
        ));
        assert!(matches!(
            doc.render(0, 1.0, 45),
            Err(EngineError::InvalidRotation)
        ));
        assert!(matches!(
            doc.render(0, 64.0, 0),
            Err(EngineError::TooLarge { .. })
        ));
    }

    #[test]
    fn huge_media_box_is_rejected_instead_of_allocated() {
        let pdf = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1000000000 1000000000] >>",
        ]);
        let doc = PdfDocument::from_bytes(&pdf).unwrap();
        assert!(matches!(
            doc.render(0, 1.0, 0),
            Err(EngineError::TooLarge { .. })
        ));
    }

    #[test]
    fn non_pdf_input_is_rejected() {
        assert!(matches!(
            PdfDocument::from_bytes(b""),
            Err(EngineError::NotPdf)
        ));
        assert!(matches!(
            PdfDocument::from_bytes(b"This is a plain text file with a .pdf extension.\n"),
            Err(EngineError::NotPdf)
        ));
    }

    #[test]
    fn garbage_after_the_header_does_not_panic() {
        let result = PdfDocument::from_bytes(b"%PDF-1.7\n\x00\xff garbage without objects");
        if let Ok(doc) = result {
            assert!(doc.page_count().is_ok());
        }
    }

    /// The JavaScript engine is compiled out: even an explicit request cannot enable it.
    /// (Test-only call; production code must never call `enable_js`.)
    #[test]
    fn javascript_is_compiled_out() {
        let pdf = build_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R /OpenAction << /S /JavaScript /JS (app.alert\\(1\\)) >> >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        ]);
        let mut doc = mupdf::pdf::PdfDocument::from_bytes(&pdf).unwrap();
        doc.enable_js().unwrap();
        assert!(!doc.is_js_supported().unwrap());
    }
}
