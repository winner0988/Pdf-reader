//! Delivers [`OpenEvent`]s to the frontend over the channel it passes to
//! `subscribe_open_events`.
//!
//! A channel rather than Tauri events: listening to events needs the `core:event` permission,
//! which would also let the page receive Tauri's own drag-and-drop events, and those carry full
//! paths. With no event permission granted, paths cannot reach the WebView.

use std::sync::Mutex;

use ipc_contract::types::{DocumentInfo, OpenEvent};
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
    /// it missed: queued events, or else the document that is already open.
    pub fn subscribe(&self, channel: Channel<OpenEvent>, current: Option<DocumentInfo>) {
        let mut sink = self.lock();
        let queued = std::mem::take(&mut sink.queued);
        let missed = if queued.is_empty() {
            current
                .map(|info| OpenEvent::Opened {
                    info,
                    ignored_files: 0,
                })
                .into_iter()
                .collect()
        } else {
            queued
        };
        for event in missed {
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

    use ipc_contract::types::{DocumentId, PageSize, SecurityReport};
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

    fn opening(name: &str) -> OpenEvent {
        OpenEvent::Opening {
            display_name: name.to_owned(),
        }
    }

    #[test]
    fn events_before_subscribing_are_delivered_in_order() {
        let events = OpenEvents::default();
        events.send(OpenEvent::DragHover { active: true });
        events.send(opening("a.pdf"));
        events.send(opening("b.pdf"));

        let (channel, received) = recording_channel();
        events.subscribe(channel, None);
        events.send(opening("c.pdf"));

        let names: Vec<_> = received
            .lock()
            .unwrap()
            .iter()
            .map(|event| event["displayName"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["a.pdf", "b.pdf", "c.pdf"]);
    }

    #[test]
    fn the_queue_is_bounded() {
        let events = OpenEvents::default();
        for index in 0..MAX_QUEUED + 5 {
            events.send(opening(&format!("{index}.pdf")));
        }
        let (channel, received) = recording_channel();
        events.subscribe(channel, None);
        let received = received.lock().unwrap();
        assert_eq!(received.len(), MAX_QUEUED);
        assert_eq!(received[0]["displayName"], "5.pdf");
    }

    #[test]
    fn a_reloaded_page_gets_the_open_document() {
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
        events.subscribe(channel, Some(info));
        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["kind"], "opened");
        assert_eq!(received[0]["info"]["doc"], 3);
        assert_eq!(received[0]["ignoredFiles"], 0);
    }
}
