//! The fan-out of changes to live SSE streams.
//!
//! - Per match: one `broadcast` channel per match id carrying fresh views, for the score and
//!   view streams.
//! - Per user: one channel per owner id carrying a bare "something changed" signal, for the
//!   owned-matches list stream, which re-reads the list on it. A match the user owns was
//!   created, or a command was applied to it.
//! - Per event: one channel per event id carrying a bare "something changed" signal, for the
//!   coordinator's event stream, which re-reads the event on it: a coordinator route changed
//!   the event, a command was applied to one of its matches, or a court phone came or went.
//!
//! [`CourtPhones`] counts the court phones connected to each court, in memory only.
//!
//! A channel is created on the first subscribe and dropped as soon as its last
//! [`Subscription`] goes (and, as a backstop, when a publish finds no receivers), so the maps
//! only hold what someone is watching. The lock is a plain `std::sync::Mutex`: it guards map
//! operations and a non-blocking `send` only, never an await.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::commands::MatchView;

/// How many messages a slow stream may fall behind before it lags (and re-reads the store).
pub const CAPACITY: usize = 16;

/// One map of channels by key, carrying `T`.
#[derive(Debug)]
struct Channels<T> {
    map: Mutex<HashMap<String, broadcast::Sender<T>>>,
}

impl<T> Default for Channels<T> {
    fn default() -> Self {
        Channels {
            map: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: Clone> Channels<T> {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, broadcast::Sender<T>>> {
        // Nothing here can leave the map half-updated, so a poisoned lock is still usable.
        self.map.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn subscribe(&self, key: &str) -> broadcast::Receiver<T> {
        self.lock()
            .entry(key.to_owned())
            .or_insert_with(|| broadcast::channel(CAPACITY).0)
            .subscribe()
    }

    fn publish(&self, key: &str, value: T) {
        let mut map = self.lock();
        let Some(tx) = map.get(key) else {
            return;
        };
        // `send` fails only when there are no receivers, which the check below handles.
        let _ = tx.send(value);
        if tx.receiver_count() == 0 {
            map.remove(key);
        }
    }

    fn release(&self, key: &str) {
        let mut map = self.lock();
        if map.get(key).is_some_and(|tx| tx.receiver_count() == 0) {
            map.remove(key);
        }
    }
}

#[derive(Debug, Default)]
pub struct Hub {
    matches: Channels<Arc<MatchView>>,
    users: Channels<()>,
    events: Channels<()>,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// Watch a match. The returned subscription sees every view published after this call.
    pub fn subscribe(self: &Arc<Self>, match_id: &str) -> Subscription {
        Subscription {
            rx: Some(self.matches.subscribe(match_id)),
            hub: Arc::clone(self),
            key: match_id.to_owned(),
            channels: |hub| &hub.matches,
        }
    }

    /// Send a fresh view to everyone watching its match. A match nobody watches is skipped.
    pub fn publish(&self, view: MatchView) {
        let id = view.id.clone();
        self.matches.publish(&id, Arc::new(view));
    }

    /// Drop the match's entry. The sender goes with it, so every subscriber's `recv` returns
    /// `Closed` and its stream ends. Used when the match is purged.
    pub fn close(&self, match_id: &str) {
        self.matches.lock().remove(match_id);
    }

    /// Whether anyone is watching the match.
    pub fn is_watched(&self, match_id: &str) -> bool {
        self.matches.lock().contains_key(match_id)
    }

    /// The number of matches with a live channel.
    pub fn len(&self) -> usize {
        self.matches.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.lock().is_empty()
    }

    /// Watch a user's owned matches. The subscription gets a `()` for every change notified
    /// after this call.
    pub fn subscribe_user(self: &Arc<Self>, user_id: &str) -> Subscription<()> {
        Subscription {
            rx: Some(self.users.subscribe(user_id)),
            hub: Arc::clone(self),
            key: user_id.to_owned(),
            channels: |hub| &hub.users,
        }
    }

    /// Tell the user's list streams that one of their matches changed. Nobody watching: skipped.
    pub fn notify_user(&self, user_id: &str) {
        self.users.publish(user_id, ());
    }

    /// Whether anyone is watching the user's owned matches.
    pub fn is_user_watched(&self, user_id: &str) -> bool {
        self.users.lock().contains_key(user_id)
    }

    /// Watch an event. The subscription gets a `()` for every change notified after this call.
    pub fn subscribe_event(self: &Arc<Self>, event_id: &str) -> Subscription<()> {
        Subscription {
            rx: Some(self.events.subscribe(event_id)),
            hub: Arc::clone(self),
            key: event_id.to_owned(),
            channels: |hub| &hub.events,
        }
    }

    /// Tell the event's streams that it changed. Nobody watching: skipped.
    pub fn notify_event(&self, event_id: &str) {
        self.events.publish(event_id, ());
    }
}

/// How many court phones (court streams opened with a court's scorer token) are connected to
/// each court, for the coordinator's "Phone connected". In memory only: a restart starts at 0,
/// and the phones' reconnects count again. The lock guards map operations only, never an await.
#[derive(Debug, Default)]
pub struct CourtPhones {
    counts: Mutex<HashMap<String, usize>>,
}

impl CourtPhones {
    pub fn new() -> CourtPhones {
        CourtPhones::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, usize>> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Count one more phone on `court_id` until the returned guard drops. Both the join and the
    /// drop notify the event, so its coordinator sees the phone come and go.
    pub fn join(self: &Arc<Self>, hub: &Arc<Hub>, event_id: &str, court_id: &str) -> PhoneGuard {
        *self.lock().entry(court_id.to_owned()).or_insert(0) += 1;
        hub.notify_event(event_id);
        PhoneGuard {
            phones: Arc::clone(self),
            hub: Arc::clone(hub),
            event_id: event_id.to_owned(),
            court_id: court_id.to_owned(),
        }
    }

    /// The phones connected to `court_id` right now.
    pub fn count(&self, court_id: &str) -> usize {
        self.lock().get(court_id).copied().unwrap_or(0)
    }
}

/// One connected court phone; dropping it takes the phone off the count.
#[derive(Debug)]
pub struct PhoneGuard {
    phones: Arc<CourtPhones>,
    hub: Arc<Hub>,
    event_id: String,
    court_id: String,
}

impl Drop for PhoneGuard {
    fn drop(&mut self) {
        {
            let mut counts = self.phones.lock();
            if let Some(n) = counts.get_mut(&self.court_id) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    counts.remove(&self.court_id);
                }
            }
        }
        self.hub.notify_event(&self.event_id);
    }
}

/// A receiver for one match (or, with `T = ()`, one user). Dropping the last one for a key
/// removes its hub entry.
pub struct Subscription<T: Clone = Arc<MatchView>> {
    rx: Option<broadcast::Receiver<T>>,
    hub: Arc<Hub>,
    key: String,
    /// Which of the hub's maps `key` lives in.
    channels: fn(&Hub) -> &Channels<T>,
}

impl<T: Clone> std::fmt::Debug for Subscription<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl<T: Clone> Subscription<T> {
    pub async fn recv(&mut self) -> Result<T, RecvError> {
        match &mut self.rx {
            Some(rx) => rx.recv().await,
            None => Err(RecvError::Closed),
        }
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match &mut self.rx {
            Some(rx) => rx.try_recv(),
            None => Err(TryRecvError::Closed),
        }
    }
}

impl<T: Clone> Drop for Subscription<T> {
    fn drop(&mut self) {
        // Drop the receiver first so `release` sees the count without it.
        drop(self.rx.take());
        (self.channels)(&self.hub).release(&self.key);
    }
}
