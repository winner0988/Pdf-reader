//! Only for the OCR POC of ADR 0015 (examples/ocr_probe.rs, examples/tesseract_probe.rs); the
//! worker itself is unaffected.
//! - windows-core imports ole32, which imports user32, which cannot load when win32k is
//!   disabled, as in the worker's sandbox. Delay-loaded, ole32 is only loaded if called, which
//!   the Windows OCR probe avoids.
//! - With the `tesseract-poc` feature, the Tesseract probe links the static libraries that
//!   scripts/ocr-poc/build-tesseract.ps1 installs in `TESSERACT_POC_DIR`.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=TESSERACT_POC_DIR");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo::rustc-link-arg-examples=/DELAYLOAD:ole32.dll");
        println!("cargo::rustc-link-arg-examples=delayimp.lib");
    }
    if std::env::var_os("CARGO_FEATURE_TESSERACT_POC").is_some() {
        let dir = std::env::var("TESSERACT_POC_DIR").expect(
            "the tesseract-poc feature needs TESSERACT_POC_DIR: the install folder of \
             scripts/ocr-poc/build-tesseract.ps1 (target/ocr-poc/install)",
        );
        let lib = std::path::Path::new(&dir).join("lib");
        for name in ["tesseract55.lib", "leptonica-1.87.0.lib"] {
            println!(
                "cargo::rustc-link-arg-examples={}",
                lib.join(name).display()
            );
        }
    }
}
