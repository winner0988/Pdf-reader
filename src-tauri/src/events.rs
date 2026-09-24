//! Delivers [`OpenEvent`]s to the frontend over the channel it passes to
//! `subscribe_open_events`.
//!
//! A channel rather than Tauri events: listening to events needs the `core:event` permission,
//! which would also let the page receive Tauri's own drag-and-drop events, and those carry full
//! paths. With no event permission granted, paths cannot reach the WebView.

use std::sync::Mutex;

use ipc_contract::types::OpenEvent;
use tauri::ipc::Channel;

/// Events kept while no frontend is subscribed (e.g. a command-line open during start-up).
const MAX_QUEUED: usize = 16;

#[derive(Default)]
pub struct OpenEvents {
    sink: Mutex<Sink>,
}

#[derive(Default)]
struct Sink {
    channel: Option<Channel<OpenEvent>>,
    queued: Vec<OpenEvent>,
}

impl OpenEvents {
    pub fn send(&self, event: OpenEvent) {
        let mut sink = self.lock();
        match &sink.channel {
            Some(channel) => {
                if channel.send(event).is_err() {
                    sink.channel = None;
                }
            }
            // Hover state is only meaningful while it happens.
            None if matches!(event, OpenEvent::DragHover { .. }) => {}
            None => {
                if sink.queued.len() == MAX_QUEUED {
                    sink.queued.remove(0);
                }
                sink.queued.push(event);
            }
        }
    }

    /// Makes `channel` the receiver (a reloaded page replaces the previous one) and delivers what
    /// it missed: every tab as it is now (`snapshot`, e.g. `Documents::snapshot`), then any
    /// queued tab-limit notices. The snapshot is taken under this lock, so no event slips in
    /// between. Tab events are recorded before they are sent, so the queued ones are already in
    /// the snapshot; one sent right after it only repeats the same state, which is harmless.
    pub fn subscribe(
        &self,
        channel: Channel<OpenEvent>,
        snapshot: impl FnOnce() -> Vec<OpenEvent>,
    ) {
        let mut sink = self.lock();
        let notices = std::mem::take(&mut sink.queued)
            .into_iter()
            .filter(|event| matches!(event, OpenEvent::TabLimit { .. }));
        for event in snapshot().into_iter().chain(notices) {
            // A failure here surfaces on the next send.
            let _ = channel.send(event);
        }
        sink.channel = Some(channel);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Sink> {
        self.sink
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ipc_contract::types::{DocumentId, DocumentInfo, PageSize, SecurityReport, TabId};
    use tauri::ipc::InvokeResponseBody;

    use super::*;

    fn recording_channel() -> (Channel<OpenEvent>, Arc<Mutex<Vec<serde_json::Value>>>) {
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = received.clone();
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("expected JSON");
            };
            sink.lock()
                .unwrap()
                .push(serde_json::from_str(&json).unwrap());
            Ok(())
        });
        (channel, received)
    }

    fn opening(tab: u32, name: &str) -> OpenEvent {
        OpenEvent::Opening {
            tab: TabId(tab),
            display_name: name.to_owned(),
        }
    }

    #[test]
    fn a_new_page_gets_every_tab_from_the_snapshot_then_the_queued_notices() {
        let events = OpenEvents::default();
        events.send(OpenEvent::DragHover { active: true });
        // Already in the snapshot: not delivered twice.
        events.send(opening(1, "a.pdf"));
        events.send(OpenEvent::TabLimit { ignored_files: 3 });

        let (channel, received) = recording_channel();
        events.subscribe(channel, || vec![opening(1, "a.pdf"), opening(2, "b.pdf")]);
        events.send(opening(3, "c.pdf"));

        let received = received.lock().unwrap();
        let kinds: Vec<_> = received
            .iter()
            .map(|event| {
                event["displayName"]
                    .as_str()
                    .map_or_else(|| event["kind"].to_string(), str::to_owned)
            })
            .collect();
        assert_eq!(kinds, ["a.pdf", "b.pdf", "\"tabLimit\"", "c.pdf"]);
        assert_eq!(received[2]["ignoredFiles"], 3);
    }

    #[test]
    fn the_queue_is_bounded() {
        let events = OpenEvents::default();
        for index in 0..MAX_QUEUED + 5 {
            events.send(OpenEvent::TabLimit {
                ignored_files: u32::try_from(index).unwrap(),
            });
        }
        let (channel, received) = recording_channel();
        events.subscribe(channel, Vec::new);
        let received = received.lock().unwrap();
        assert_eq!(received.len(), MAX_QUEUED);
        assert_eq!(received[0]["ignoredFiles"], 5);
    }

    #[test]
    fn a_reloaded_page_gets_the_open_documents() {
        let events = OpenEvents::default();
        let info = DocumentInfo {
            doc: DocumentId(3),
            display_name: "a.pdf".to_owned(),
            pages: vec![PageSize {
                width_pt: 612.0,
                height_pt: 792.0,
            }],
            has_outline: false,
            security: SecurityReport::default(),
        };
        let (channel, received) = recording_channel();
        events.subscribe(channel, || {
            vec![OpenEvent::Opened {
                tab: TabId(4),
                info,
            }]
        });
        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["kind"], "opened");
        assert_eq!(received[0]["tab"], 4);
        assert_eq!(received[0]["info"]["doc"], 3);
    }
}
