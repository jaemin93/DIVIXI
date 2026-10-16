//! The app's events, for other devices.
//!
//! Every event the UI listens to is numbered and kept in a ring; a device
//! that reconnects asks for what came after the last number it saw, and is
//! told to reload when the ring no longer reaches back that far. Terminal
//! output is live only ([`LIVE`]): unnumbered, never kept, and dropped
//! rather than waited for when a device falls behind (a shell redraws).
//!
//! The numbers start from the clock each time keeping starts ([`Hub::start`]),
//! so they only ever grow, across restarts too. A device holding a number
//! from before a restart is then behind the ring, and reloads, instead of
//! being sent the new numbers as though they followed its own.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Listener, Runtime};
use tokio::sync::broadcast;

/// Events the UI listens to, kept for catching up.
pub const EVENTS: &[&str] = &[
    "agent",
    "agent_download",
    "decision",
    "design",
    "design_extract",
    "conductor_handoff",
    "track_saved",
    "track_deleted",
    "parked",
    "routines",
    "routine_done",
    "routine_progress",
    "plan",
    "knowledge",
    "knowledge-removed",
    "knowledge-embedding",
];

/// The terminal's events: sent as they come, not kept.
pub const LIVE: &[&str] = &["term", "term_exit"];

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
    on: AtomicBool,
    ring: parking_lot::Mutex<(u64, VecDeque<Frame>)>,
    tx: broadcast::Sender<Frame>,
    live: broadcast::Sender<Arc<str>>,
}

impl Default for Hub {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(1024);
        let (live, _) = broadcast::channel(4096);
        Self { on: AtomicBool::new(false), ring: parking_lot::Mutex::new((0, VecDeque::new())), tx, live }
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
    /// Start keeping events: the server is up. The numbers move on to the
    /// clock, in microseconds, past any given out before -- by this process
    /// or one before it -- and what was kept goes, so a device that comes
    /// back holding a number from an earlier stretch is told to reload
    /// rather than miss what happened in between, unkept.
    pub fn start(&self) {
        self.start_at(clock());
    }

    fn start_at(&self, now: u64) {
        let mut ring = self.ring.lock();
        if self.on.swap(true, Ordering::Relaxed) {
            return;
        }
        // One number at least is skipped, whatever the clock says, so the
        // last one given before is never followed on from: a socket left
        // open across it sees the gap and is told to reload (server.rs).
        ring.0 = (ring.0 + 1).max(now);
        ring.1.clear();
    }

    /// Stop keeping events: the server is down.
    pub fn stop(&self) {
        self.on.store(false, Ordering::Relaxed);
    }

    pub fn push(&self, event: &str, payload: &str) {
        if !self.on.load(Ordering::Relaxed) {
            return;
        }
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
        // Sent under the lock, so frames go out in their numbers' order: two
        // events at once from two threads could otherwise be sent n+1 then
        // n, and a socket that has sent n+1 drops n as already sent. Sending
        // does not wait.
        let _ = self.tx.send(frame);
    }

    /// A live event: `{"event":"…","payload":…}`, to whoever listens now.
    pub fn push_live(&self, event: &str, payload: &str) {
        if !self.on.load(Ordering::Relaxed) || self.live.receiver_count() == 0 {
            return;
        }
        let payload = if payload.is_empty() { "null" } else { payload };
        let _ = self.live.send(format!("{{\"event\":{},\"payload\":{payload}}}", serde_json::Value::String(event.into())).into());
    }

    pub fn subscribe_live(&self) -> broadcast::Receiver<Arc<str>> {
        self.live.subscribe()
    }

    /// The newest number.
    #[cfg(test)]
    pub fn last(&self) -> u64 {
        self.ring.lock().0
    }

    /// Subscribe, and what came after `since` (0: nothing to catch up on),
    /// with the newest number: where the device is once caught up.
    pub fn subscribe(&self, since: u64) -> (broadcast::Receiver<Frame>, CatchUp, u64) {
        let ring = self.ring.lock();
        let rx = self.tx.subscribe();
        let newest = ring.0;
        if since == 0 || since == newest {
            return (rx, CatchUp::Frames(Vec::new()), newest);
        }
        let oldest = ring.1.front().map(|f| f.seq).unwrap_or(newest + 1);
        // Past the newest: a number this hub never gave (the clock was set
        // back across a restart). As lost as one too old.
        if since > newest || since + 1 < oldest {
            return (rx, CatchUp::Reset, newest);
        }
        (rx, CatchUp::Frames(ring.1.iter().filter(|f| f.seq > since).cloned().collect()), newest)
    }
}

/// The least number a hub numbering from the clock gives: 2020, in
/// microseconds. An older divixi-server numbers from 1 and never comes near.
pub const CLOCKED: u64 = 1_577_836_800_000_000;

/// Now, in microseconds since 1970: more than any number given out before,
/// unless events came faster than one a microsecond. Under 2^53 for two
/// centuries, so a page's JavaScript holds it exactly.
fn clock() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}

/// Feed the hub from the app's events.
pub fn install<R: Runtime>(app: &AppHandle<R>, hub: Arc<Hub>) {
    for &name in EVENTS {
        let hub = hub.clone();
        app.listen_any(name, move |event| hub.push(name, event.payload()));
    }
    for &name in LIVE {
        let hub = hub.clone();
        app.listen_any(name, move |event| hub.push_live(name, event.payload()));
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
        hub.start_at(1000);
        hub.push("agent", "{\"a\":1}");
        hub.push("decision", "");
        let (_rx, catch, newest) = hub.subscribe(1001);
        let CatchUp::Frames(frames) = catch else { panic!("in range") };
        assert_eq!(frames.len(), 1);
        assert_eq!(&*frames[0].text, "{\"seq\":1002,\"event\":\"decision\",\"payload\":null}");
        assert_eq!(newest, 1002);
        assert!(matches!(hub.subscribe(1002).1, CatchUp::Frames(f) if f.is_empty()), "caught up");
        for _ in 0..RING + 5 {
            hub.push("agent", "{}");
        }
        assert!(matches!(hub.subscribe(1001).1, CatchUp::Reset), "fell out of the ring");
        let (_rx, catch, newest) = hub.subscribe(0);
        assert!(matches!(catch, CatchUp::Frames(f) if f.is_empty()), "a fresh device starts now");
        assert_eq!(newest, hub.last(), "and is told where now is");
    }

    /// Events from several threads at once go out in their numbers' order:
    /// a socket drops any frame numbered at or below one it has sent.
    #[test]
    fn frames_are_sent_in_order() {
        let hub = Arc::new(Hub::default());
        hub.start_at(1000);
        let mut rx = hub.tx.subscribe();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let hub = hub.clone();
                std::thread::spawn(move || {
                    for _ in 0..200 {
                        hub.push("agent", "{}");
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let mut last = 1000;
        for _ in 0..800 {
            let f = rx.try_recv().expect("all 800 sent");
            assert_eq!(f.seq, last + 1);
            last = f.seq;
        }
    }

    /// The server restarted while a device was away: its number is from the
    /// earlier process, and the new one may have numbered well past it since.
    #[test]
    fn a_number_from_before_a_restart_reloads() {
        let before = Hub::default();
        before.start_at(1000);
        before.push("decision", "{}");
        let seen = before.last();

        let after = Hub::default();
        after.start_at(5000);
        assert!(matches!(after.subscribe(seen).1, CatchUp::Reset), "nothing kept yet");
        for _ in 0..10 {
            after.push("decision", "{}");
        }
        assert!(matches!(after.subscribe(seen).1, CatchUp::Reset), "the new numbers do not follow on from it");
    }

    #[test]
    fn a_number_never_given_reloads() {
        let hub = Hub::default();
        hub.start_at(1000);
        hub.push("decision", "{}");
        let (_rx, catch, newest) = hub.subscribe(hub.last() + 5);
        assert!(matches!(catch, CatchUp::Reset));
        assert_eq!(newest, 1001);
    }

    /// Off and on again (the desktop app's switch): what happened while off
    /// was not kept, so a device from before reloads.
    #[test]
    fn a_number_from_before_the_server_went_off_reloads() {
        let hub = Hub::default();
        hub.start_at(1000);
        hub.push("decision", "{}");
        let seen = hub.last();
        hub.stop();
        hub.push("decision", "{}");
        // A clock behind the numbers (set back while off).
        hub.start_at(900);
        assert!(matches!(hub.subscribe(seen).1, CatchUp::Reset), "nothing kept yet");
        hub.push("decision", "{}");
        assert!(hub.last() > seen, "numbers never go back");
        assert!(matches!(hub.subscribe(seen).1, CatchUp::Reset), "the new numbers do not follow on from it");
        let last = hub.last();
        hub.start_at(9000);
        assert_eq!(hub.last(), last, "starting again while on changes nothing");
    }

    /// A subscriber is sent the numbers after the newest, one after another,
    /// until the server goes off and on under it: then one is skipped, which
    /// is how a socket left open knows (server.rs).
    #[test]
    fn a_subscriber_sees_a_gap_only_across_off_and_on() {
        let hub = Hub::default();
        hub.start_at(1000);
        hub.push("decision", "{}");
        let (mut rx, _, newest) = hub.subscribe(0);
        hub.push("decision", "{}");
        hub.push("decision", "{}");
        assert_eq!(rx.try_recv().unwrap().seq, newest + 1);
        assert_eq!(rx.try_recv().unwrap().seq, newest + 2);
        hub.stop();
        hub.push("decision", "{}");
        // A clock behind the numbers: still a gap.
        hub.start_at(900);
        hub.push("decision", "{}");
        assert!(rx.try_recv().unwrap().seq > newest + 3);
    }

    /// A spent pairing link is told to the window showing it, and to no device.
    #[test]
    fn a_spent_link_is_not_sent_to_devices() {
        assert!(!EVENTS.contains(&super::super::server::PAIRED_EVENT));
        assert!(!LIVE.contains(&super::super::server::PAIRED_EVENT));
    }

    #[test]
    fn live_events_are_sent_not_kept() {
        let hub = Hub::default();
        hub.start();
        let before = hub.last();
        let mut rx = hub.subscribe_live();
        hub.push_live("term", "{\"id\":1,\"data\":\"hi\"}");
        assert_eq!(&*rx.try_recv().unwrap(), "{\"event\":\"term\",\"payload\":{\"id\":1,\"data\":\"hi\"}}");
        assert_eq!(hub.last(), before, "not numbered, not kept");
    }
}
