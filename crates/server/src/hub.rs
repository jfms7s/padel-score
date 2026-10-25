//! The per-match fan-out of fresh views to live SSE streams.
//!
//! One `broadcast` channel per match id, created on the first subscribe. An entry is dropped as
//! soon as its last [`Subscription`] goes (and, as a backstop, when a publish finds no
//! receivers), so the map only holds matches someone is watching. The lock is a plain
//! `std::sync::Mutex`: it guards map operations and a non-blocking `send` only, never an await.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::broadcast;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};

use crate::commands::MatchView;

/// How many views a slow stream may fall behind before it lags (and re-reads the store).
pub const CAPACITY: usize = 16;

type Sender = broadcast::Sender<Arc<MatchView>>;

#[derive(Debug, Default)]
pub struct Hub {
    channels: Mutex<HashMap<String, Sender>>,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    fn channels(&self) -> MutexGuard<'_, HashMap<String, Sender>> {
        // Nothing here can leave the map half-updated, so a poisoned lock is still usable.
        self.channels.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Watch a match. The returned subscription sees every view published after this call.
    pub fn subscribe(self: &Arc<Self>, match_id: &str) -> Subscription {
        let rx = self
            .channels()
            .entry(match_id.to_owned())
            .or_insert_with(|| broadcast::channel(CAPACITY).0)
            .subscribe();
        Subscription {
            rx: Some(rx),
            hub: Arc::clone(self),
            match_id: match_id.to_owned(),
        }
    }

    /// Send a fresh view to everyone watching its match. A match nobody watches is skipped.
    pub fn publish(&self, view: MatchView) {
        let mut channels = self.channels();
        let Some(tx) = channels.get(&view.id) else {
            return;
        };
        let id = view.id.clone();
        // `send` fails only when there are no receivers, which the check below handles.
        let _ = tx.send(Arc::new(view));
        if tx.receiver_count() == 0 {
            channels.remove(&id);
        }
    }

    /// The number of matches with a live channel.
    pub fn len(&self) -> usize {
        self.channels().len()
    }

    pub fn is_empty(&self) -> bool {
        self.channels().is_empty()
    }

    fn release(&self, match_id: &str) {
        let mut channels = self.channels();
        if channels
            .get(match_id)
            .is_some_and(|tx| tx.receiver_count() == 0)
        {
            channels.remove(match_id);
        }
    }
}

/// A receiver for one match. Dropping the last one for a match removes its hub entry.
#[derive(Debug)]
pub struct Subscription {
    rx: Option<broadcast::Receiver<Arc<MatchView>>>,
    hub: Arc<Hub>,
    match_id: String,
}

impl Subscription {
    pub async fn recv(&mut self) -> Result<Arc<MatchView>, RecvError> {
        match &mut self.rx {
            Some(rx) => rx.recv().await,
            None => Err(RecvError::Closed),
        }
    }

    pub fn try_recv(&mut self) -> Result<Arc<MatchView>, TryRecvError> {
        match &mut self.rx {
            Some(rx) => rx.try_recv(),
            None => Err(TryRecvError::Closed),
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // Drop the receiver first so `release` sees the count without it.
        drop(self.rx.take());
        self.hub.release(&self.match_id);
    }
}
