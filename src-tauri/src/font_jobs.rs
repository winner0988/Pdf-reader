//! Document font jobs (B5-04): asynchronous font discovery tasks that can be cancelled.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ipc_contract::types::RequestId;

/// Tracks active font discovery jobs so they can be cancelled when a dialog closes or tab switches.
#[derive(Default)]
pub struct FontJobs {
    running: Mutex<HashMap<RequestId, Arc<AtomicBool>>>,
}

impl FontJobs {
    /// Signals the font discovery job for `request` to stop.
    pub fn cancel(&self, request: RequestId) {
        if let Some(stop) = self.lock().get(&request) {
            stop.store(true, Ordering::SeqCst);
        }
    }

    /// Registers `request` as running and returns a cancellation flag.
    pub fn start(&self, request: RequestId) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.lock().insert(request, flag.clone());
        flag
    }

    /// Unregisters `request` after completion or cancellation.
    pub fn finish(&self, request: RequestId) {
        self.lock().remove(&request);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RequestId, Arc<AtomicBool>>> {
        self.running
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}
