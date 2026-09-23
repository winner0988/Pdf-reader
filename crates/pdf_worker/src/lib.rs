//! `pdf_worker`: the isolated PDF engine process (ADR 0008).
//!
//! This is the only crate that links MuPDF. The binary (`main.rs`) will speak the IPC contract
//! over stdio (MVP-04); [`engine`] is the thin, safe wrapper around MuPDF it uses.

pub mod engine;
