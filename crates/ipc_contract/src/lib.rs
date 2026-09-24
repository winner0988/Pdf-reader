//! IPC contract between the frontend (WebView), the main process and `pdf_worker` (ADR 0008).
//!
//! This crate is the single source of truth for every message that crosses a process boundary:
//!
//! - [`types`]: values the frontend sees (JSON over Tauri commands and channels). TypeScript
//!   declarations for them are generated into `src/ipc/generated/contract.ts`.
//! - [`worker`]: messages between the main process and `pdf_worker` (postcard over stdio).
//! - [`frame`]: length-prefixed framing with a size limit checked before allocation.
//! - [`raster`]: the binary layout used to hand rendered pages to the frontend.
//! - [`validate`]: bounds checks for everything that originates in the untrusted worker.
//!
//! Design notes and message tables live in `docs/architecture/ipc-contract.md`.
//!
//! Invariants enforced here (and by tests):
//! - No frontend-visible type carries a file path. Documents are identified by [`types::DocumentId`].
//! - There is no message to read an arbitrary file, run a command or open an arbitrary URL.
//! - Every collection and string coming from the worker has an upper bound in [`limits`].

pub mod frame;
pub mod limits;
pub mod raster;
pub mod types;
pub mod typescript;
pub mod validate;
pub mod worker;

/// Version of the IPC contract. Main and worker refuse to talk when their versions differ.
///
/// Contract v0 is pre-stable: any change may be breaking, so both sides ship together.
pub const PROTOCOL_VERSION: u32 = 0;
