//! One Server-Sent Events stream for everything that changes: job progress, song updates,
//! model downloads. SSE reconnects by itself when a phone wakes up, and a single stream
//! leaves the browser's per-host connection budget to the audio requests.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use futures_util::stream::{self, Stream, StreamExt};
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

use crate::app::AppState;
use crate::jobs::Event;

fn to_sse(event: &Event) -> SseEvent {
    SseEvent::default().json_data(event).unwrap_or_else(|_| SseEvent::default().comment("encode error"))
}

pub async fn stream(State(st): State<AppState>) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    // Subscribe first, then snapshot, so nothing falls between the two.
    let live = BroadcastStream::new(st.events.subscribe());
    let snapshot: Vec<Event> =
        st.db.recent_jobs().unwrap_or_default().into_iter().map(|job| Event::Job { job }).collect();

    let initial = stream::iter(snapshot).map(|e| Ok(to_sse(&e)));
    let updates = live.map(|item| {
        Ok(match item {
            Ok(event) => to_sse(&event),
            // The client fell behind: tell it to refetch instead of guessing what it missed.
            Err(BroadcastStreamRecvError::Lagged(_)) => to_sse(&Event::System),
        })
    });
    Sse::new(initial.chain(updates)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
