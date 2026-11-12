use crate::throttle::Throttle;
use crate::{Notification, Notify};
use hoshi_core::{StreamDelta, StreamKind};
use std::sync::Arc;
use std::time::Duration;

const STREAM_EVERY: Duration = Duration::from_millis(50);

/// Text still streaming reaches the UI in batches, at most once per `STREAM_EVERY`. An event
/// sends what waits first, so the UI never gets text after the item it belongs to completed.
pub(crate) fn batch_streams(notify: Notify) -> Notify {
    let streams = {
        let notify = notify.clone();
        Throttle::new(STREAM_EVERY, merge, move |deltas| {
            notify(Notification::Stream(deltas));
        })
    };
    Arc::new(move |notification| match notification {
        Notification::Stream(deltas) => streams.offer(deltas),
        Notification::Event(_) => {
            streams.flush();
            notify(notification);
        }
        other => notify(other),
    })
}

/// Text is appended to what waits for its item; a tool's output is its whole output so far,
/// so it replaces the one before.
fn merge(waiting: &mut Vec<StreamDelta>, more: Vec<StreamDelta>) {
    for delta in more {
        let same = waiting.iter_mut().find(|other| {
            other.thread_id == delta.thread_id
                && other.item_id == delta.item_id
                && other.kind == delta.kind
        });
        match same {
            Some(same) if delta.kind == StreamKind::ToolOutput => same.delta = delta.delta,
            Some(same) => same.delta.push_str(&delta.delta),
            None => waiting.push(delta),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::{Event, EventEnvelope};
    use std::sync::Mutex;

    fn delta(item_id: &str, kind: StreamKind, text: &str) -> StreamDelta {
        StreamDelta {
            thread_id: "t1".into(),
            item_id: item_id.into(),
            kind,
            delta: text.into(),
        }
    }

    #[test]
    fn deltas_merge_by_item_text_appended_and_output_replaced() {
        let mut waiting = vec![delta("a", StreamKind::Text, "Hel")];
        merge(
            &mut waiting,
            vec![
                delta("o", StreamKind::ToolOutput, "line 1\n"),
                delta("a", StreamKind::Text, "lo"),
                delta("a", StreamKind::Reasoning, "hm"),
                delta("o", StreamKind::ToolOutput, "line 1\nline 2\n"),
            ],
        );
        assert_eq!(
            waiting,
            [
                delta("a", StreamKind::Text, "Hello"),
                delta("o", StreamKind::ToolOutput, "line 1\nline 2\n"),
                delta("a", StreamKind::Reasoning, "hm"),
            ]
        );
    }

    #[test]
    fn an_event_goes_after_the_text_that_waited() {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let record = sent.clone();
        let notify = batch_streams(Arc::new(move |notification| {
            let seen = match notification {
                Notification::Stream(deltas) => deltas
                    .iter()
                    .map(|delta| delta.delta.clone())
                    .collect::<Vec<_>>()
                    .join("|"),
                Notification::Event(_) => "event".into(),
                _ => "other".into(),
            };
            record.lock().unwrap().push(seen);
        }));
        for text in ["a", "b", "c"] {
            notify(Notification::Stream(vec![delta(
                "i",
                StreamKind::Text,
                text,
            )]));
        }
        notify(Notification::Event(EventEnvelope {
            sequence: 1,
            event: Event::ThreadDeleted {
                thread_id: "t1".into(),
            },
        }));
        assert_eq!(*sent.lock().unwrap(), ["a", "bc", "event"]);
    }
}
