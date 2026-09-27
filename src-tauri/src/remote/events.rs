//! The app's events, for other devices.
//!
//! Every event the UI listens to is numbered and kept in a ring; a device
//! that reconnects asks for what came after the last number it saw, and is
//! told to reload when the ring no longer reaches back that far. The PC's
//! terminal output is not among them.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Listener, Runtime};
use tokio::sync::broadcast;

/// Events the UI listens to, the terminal's left out.
pub const EVENTS: &[&str] = &[
    "agent",
    "agent_download",
    "decision",
    "design",
    "design_extract",
    "conductor_handoff",
    "knowledge",
    "knowledge-removed",
    "knowledge-embedding",
];

/// Frames kept for catching up.
const RING: usize = 4000;

/// One event as sent: `{"seq":n,"event":"…","payload":…}`.
#[derive(Clone, Debug)]
pub struct Frame {
    pub seq: u64,
    pub text: Arc<str>,
}

pub struct Hub {
    /// Kept only while the server is on (divixi-server).
    pub on: AtomicBool,
    ring: parking_lot::Mutex<(u64, VecDeque<Frame>)>,
    tx: broadcast::Sender<Frame>,
}

impl Default for Hub {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(1024);
        Self { on: AtomicBool::new(false), ring: parking_lot::Mutex::new((0, VecDeque::new())), tx }
    }
}

/// What a reconnecting device gets first.
pub enum CatchUp {
    /// The frames after the one it saw.
    Frames(Vec<Frame>),
    /// Too far behind: reload.
    Reset,
}

impl Hub {
    pub fn push(&self, event: &str, payload: &str) {
        if !self.on.load(Ordering::Relaxed) {
            return;
        }
        let frame = {
            let mut ring = self.ring.lock();
            ring.0 += 1;
            let seq = ring.0;
            let payload = if payload.is_empty() { "null" } else { payload };
            let text: Arc<str> = format!("{{\"seq\":{seq},\"event\":{},\"payload\":{payload}}}", serde_json::Value::String(event.into())).into();
            let frame = Frame { seq, text };
            ring.1.push_back(frame.clone());
            if ring.1.len() > RING {
                ring.1.pop_front();
            }
            frame
        };
        let _ = self.tx.send(frame);
    }

    /// The newest number.
    pub fn last(&self) -> u64 {
        self.ring.lock().0
    }

    /// Subscribe, and what came after `since` (0: nothing to catch up on).
    pub fn subscribe(&self, since: u64) -> (broadcast::Receiver<Frame>, CatchUp) {
        let ring = self.ring.lock();
        let rx = self.tx.subscribe();
        if since == 0 || since >= ring.0 {
            return (rx, CatchUp::Frames(Vec::new()));
        }
        let oldest = ring.1.front().map(|f| f.seq).unwrap_or(ring.0 + 1);
        if since + 1 < oldest {
            return (rx, CatchUp::Reset);
        }
        (rx, CatchUp::Frames(ring.1.iter().filter(|f| f.seq > since).cloned().collect()))
    }
}

/// Feed the hub from the app's events.
pub fn install<R: Runtime>(app: &AppHandle<R>, hub: Arc<Hub>) {
    for &name in EVENTS {
        let hub = hub.clone();
        app.listen_any(name, move |event| hub.push(name, event.payload()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_and_caught_up() {
        let hub = Hub::default();
        hub.push("agent", "{}");
        assert_eq!(hub.last(), 0, "nothing is kept while the server is off");
        hub.on.store(true, Ordering::Relaxed);
        hub.push("agent", "{\"a\":1}");
        hub.push("decision", "");
        let (_rx, catch) = hub.subscribe(1);
        let CatchUp::Frames(frames) = catch else { panic!("in range") };
        assert_eq!(frames.len(), 1);
        assert_eq!(&*frames[0].text, "{\"seq\":2,\"event\":\"decision\",\"payload\":null}");
        for _ in 0..RING + 5 {
            hub.push("agent", "{}");
        }
        assert!(matches!(hub.subscribe(1).1, CatchUp::Reset), "fell out of the ring");
        assert!(matches!(hub.subscribe(0).1, CatchUp::Frames(f) if f.is_empty()), "a fresh device starts now");
    }
}
