//! `pdf_worker`: the isolated PDF engine process (ADR 0008).
//!
//! This is the only crate that links MuPDF. The binary (`main.rs`) runs [`serve::serve`] on
//! stdin/stdout inside the sandbox; [`engine`] is the thin, safe wrapper around MuPDF, and
//! [`scan`] looks for active content when a document opens.

pub mod engine;
mod handle;
pub mod scan;
pub mod search;
pub mod serve;
