//! Only for the OCR POC of ADR 0015 (examples/ocr_probe.rs); the worker itself is unaffected.
//! windows-core imports ole32, which imports user32, which cannot load when win32k is disabled,
//! as in the worker's sandbox. Delay-loaded, ole32 is only loaded if called, which the probe
//! avoids.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo::rustc-link-arg-examples=/DELAYLOAD:ole32.dll");
        println!("cargo::rustc-link-arg-examples=delayimp.lib");
    }
}
