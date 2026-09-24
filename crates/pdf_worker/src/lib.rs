//! `pdf_worker`: the isolated PDF engine process (ADR 0008).
//!
//! This is the only crate that links MuPDF. The binary (`main.rs`) runs [`serve::serve`] on
//! stdin/stdout inside the sandbox; [`engine`] is the thin, safe wrapper around MuPDF.

pub mod engine;
mod handle;
pub mod search;
pub mod serve;
