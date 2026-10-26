//! The non-stop clock's routes and its runner.
//!
//! - `GET /api/time` → `{"now_ms"}`: the server's clock, for the phones' offset. No credential.
//! - `POST /api/events/{id}/clock` `{"action","slot"}` ([`ClockAction`]) → `EventDetail`. A
//!   [`SessionUser`] route like the other event routes: another user's event is 404, a refused
//!   tap a 409 with its code.
//!
//! Both responses carry `Cache-Control: no-store` and `Referrer-Policy: no-referrer`
//! ([`accounts::secret_headers`]): the event detail holds scorer tokens, and a cached time is
//! wrong.
//!
//! [`run_clock`] is the background task: at a slot's go it clears the practice points
//! ([`fire_due`] step 1), and at its end it sends `Command::TimeUp` to every unfinished match of
//! the slot through the ordinary command path (step 2).

use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use sqlx::SqliteConnection;

use crate::AppState;
use crate::accounts;
use crate::api::{ApiError, ApiJson, ApiPath, now};
use crate::auth::SessionUser;
use crate::commands::{
    Authority, Command, CommandOutcome, CommandRequest, MatchView, apply_command, view_of,
};
use crate::competitions::non_stop::clock::{
    ClockAction, any_running_non_stop_in, clear_time_up_in, due_go_in, due_slots_in, mark_done_in,
    next_deadline_in, pending_time_up_in, went_in,
};
use crate::competitions::view::{EventDetail, event_detail};
use crate::results;
use crate::store::{
    EventState, Store, StoreError, StoredMatch, Tx, events_in, mark, match_by_id_in, touch,
};

/// While a non-stop event is running, the runner re-checks at least this often (a match of a
/// done slot that lost its time-up gets it again within this).
pub const RECHECK: Duration = Duration::from_secs(1);

/// The shortest sleep between passes, so a deadline a pass could not clear does not spin.
const MIN_SLEEP: Duration = Duration::from_millis(20);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/time", get(time))
        .route("/api/events/{id}/clock", post(clock))
        .layer(axum::middleware::map_response(accounts::secret_headers))
}

/// The current system time in unix ms (0 before the epoch).
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[derive(Debug, Serialize)]
struct Time {
    now_ms: i64,
}

async fn time() -> Json<Time> {
    Json(Time { now_ms: now_ms() })
}

#[derive(Debug, Deserialize)]
struct ClockRequest {
    action: ClockAction,
    slot: u32,
}

async fn clock(
    State(state): State<AppState>,
    SessionUser(user): SessionUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<ClockRequest>,
) -> Result<Json<EventDetail>, ApiError> {
    let owner = &user.user.id;
    state
        .store
        .clock_action(owner, &id, body.action, body.slot, now_ms())
        .await?;
    state.clock_wake.notify_one();
    state.hub.notify_event(&id);
    state.hub.notify_user(owner);
    Ok(Json(
        event_detail(&state.store, owner, &id, &state.court_phones)
            .await?
            .ok_or(ApiError::NotFound)?,
    ))
}

/// The owner of an event, for the list-stream notification.
async fn owner_of_event_in(
    conn: &mut SqliteConnection,
    event_id: &str,
) -> Result<Option<String>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT owner_user_id FROM competitions WHERE id = ?")
            .bind(event_id)
            .fetch_optional(conn)
            .await?,
    )
}

/// Clear one match's practice points inside the go's transaction: every effective event is
/// marked replaced, the revision bumped and the result projection synced. `Ok(None)` when its
/// view cannot be built (logged; the match is left as the transaction has it).
async fn clear_match_in(
    tx: &mut Tx<'_>,
    match_id: &str,
    now_secs: i64,
) -> Result<Option<MatchView>, StoreError> {
    let events = events_in(tx, match_id).await?;
    let effective: Vec<i64> = events
        .iter()
        .filter(|e| e.state == EventState::Effective)
        .map(|e| e.seq)
        .collect();
    mark(tx, match_id, &effective, EventState::Replaced).await?;
    touch(tx, match_id, now_secs).await?;
    let Some(m) = match_by_id_in(tx, match_id).await? else {
        tracing::error!(match_id, "non-stop match lost at the go");
        return Ok(None);
    };
    let events = events_in(tx, match_id).await?;
    let view = match view_of(&m, &events) {
        Ok(view) => view,
        Err(error) => {
            tracing::error!(match_id, %error, "cannot build the match view at the go");
            return Ok(None);
        }
    };
    results::sync_in(tx, &m, &view.score, now_secs).await?;
    Ok(Some(view))
}

/// The go of one slot, in one transaction: clear every match of it without a result, then
/// [`went_in`]. Returns the fresh views and the event's owner.
async fn go_slot(
    store: &Store,
    event_id: &str,
    slot: u32,
    now_ms: i64,
    now_secs: i64,
) -> Result<(Vec<MatchView>, Option<String>), StoreError> {
    let mut tx = store.begin().await?;
    let owner = owner_of_event_in(&mut tx, event_id).await?;
    let matches: Vec<String> = sqlx::query_scalar(
        "SELECT n.match_id FROM non_stop_matches n WHERE n.event_id = ? AND n.slot = ? \
         AND NOT EXISTS (SELECT 1 FROM match_results r WHERE r.match_id = n.match_id) \
         ORDER BY n.match_id",
    )
    .bind(event_id)
    .bind(slot)
    .fetch_all(&mut *tx)
    .await?;
    let mut views = Vec::new();
    for match_id in &matches {
        if let Some(view) = clear_match_in(&mut tx, match_id, now_secs).await? {
            views.push(view);
        }
    }
    went_in(&mut tx, event_id, slot, now_ms).await?;
    tx.commit().await?;
    Ok((views, owner))
}

fn notify(state: &AppState, event_id: &str, owner: Option<&str>) {
    state.hub.notify_event(event_id);
    if let Some(owner) = owner {
        state.hub.notify_user(owner);
    }
}

/// Step 1: the go of every due slot.
async fn fire_goes(state: &AppState, now_ms: i64, now_secs: i64) {
    let due = match acquire(&state.store).await {
        Ok(mut conn) => due_go_in(&mut conn, now_ms).await,
        Err(e) => Err(e),
    };
    let due = match due {
        Ok(due) => due,
        Err(error) => {
            tracing::error!(%error, "cannot read the non-stop slots due to go");
            return;
        }
    };
    for (event_id, slot) in due {
        match go_slot(&state.store, &event_id, slot, now_ms, now_secs).await {
            Ok((views, owner)) => {
                for view in views {
                    state.hub.publish(view);
                }
                notify(state, &event_id, owner.as_deref());
            }
            Err(error) => {
                tracing::error!(event_id, slot, %error, "non-stop go failed");
            }
        }
    }
}

/// Step 2a: every running slot whose time is up is marked done, its matches flagged.
async fn close_due_slots(state: &AppState, now_ms: i64) {
    let due = match acquire(&state.store).await {
        Ok(mut conn) => due_slots_in(&mut conn, now_ms).await,
        Err(e) => Err(e),
    };
    let due = match due {
        Ok(due) => due,
        Err(error) => {
            tracing::error!(%error, "cannot read the non-stop slots due to end");
            return;
        }
    };
    for (event_id, slot) in due {
        let done = async {
            let mut tx = state.store.begin().await?;
            mark_done_in(&mut tx, &event_id, slot, now_ms).await?;
            tx.commit().await?;
            Ok::<(), StoreError>(())
        }
        .await;
        match done {
            Ok(()) => state.hub.notify_event(&event_id),
            Err(error) => tracing::error!(event_id, slot, %error, "cannot end the non-stop slot"),
        }
    }
}

/// A flagged match that still needs its time-up: the match and its log head. `None` when it is
/// gone, finished, or already has its time-up (its flag just needs clearing).
async fn check_in(
    conn: &mut SqliteConnection,
    match_id: &str,
) -> Result<Option<(StoredMatch, i64)>, StoreError> {
    let Some(m) = match_by_id_in(conn, match_id).await? else {
        return Ok(None);
    };
    let finished: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM match_results WHERE match_id = ? LIMIT 1")
            .bind(match_id)
            .fetch_optional(&mut *conn)
            .await?;
    if finished.is_some() {
        return Ok(None);
    }
    let events = events_in(conn, match_id).await?;
    let view = view_of(&m, &events)
        .map_err(|e| StoreError::Corrupt(format!("stored effective events do not replay: {e}")))?;
    let has_time_up = view
        .events
        .iter()
        .any(|e| e.event == padel_scoring::Event::TimeUp);
    if view.score.time_up || has_time_up {
        return Ok(None);
    }
    Ok(Some((m, view.head_seq)))
}

/// Call time on one flagged match. Returns whether a `TimeUp` was applied; an error leaves the
/// flag set for the next pass.
async fn time_up_match(
    state: &AppState,
    match_id: &str,
    now_secs: i64,
) -> Result<bool, StoreError> {
    let check = {
        let mut conn = acquire(&state.store).await?;
        check_in(&mut conn, match_id).await?
    };
    let Some((m, head_seq)) = check else {
        let mut conn = acquire(&state.store).await?;
        clear_time_up_in(&mut conn, match_id).await?;
        return Ok(false);
    };
    let req = CommandRequest {
        client_id: format!("time_up:{match_id}:{head_seq}"),
        batch: false,
        base_revision: m.revision,
        commands: vec![Command::TimeUp],
    };
    let outcome = apply_command(&state.store, &m, req, now_secs, Authority::Owner).await?;
    let applied = match &outcome {
        CommandOutcome::Applied(_) => true,
        CommandOutcome::Rejected { kind, .. } => {
            tracing::warn!(match_id, ?kind, "time-up refused by the engine");
            false
        }
        CommandOutcome::Replayed { .. } => false,
        CommandOutcome::Conflict(_) | CommandOutcome::Locked(_) | CommandOutcome::Invalid(_) => {
            tracing::warn!(match_id, "time-up not applied; retried on the next pass");
            return Ok(false);
        }
    };
    if let CommandOutcome::Applied(view) = outcome {
        state.hub.publish(view);
        let event_id: Option<String> =
            sqlx::query_scalar("SELECT event_id FROM non_stop_matches WHERE match_id = ?")
                .bind(match_id)
                .fetch_optional(state.store.pool())
                .await?;
        if let Some(event_id) = event_id {
            notify(state, &event_id, m.owner_user_id.as_deref());
        }
    }
    {
        let mut conn = acquire(&state.store).await?;
        clear_time_up_in(&mut conn, match_id).await?;
    }
    Ok(applied)
}

/// Step 2b: every flagged match of a slot that has gone gets its time-up, unless it is
/// finished or already has one. Returns how many were applied.
async fn fire_time_ups(state: &AppState, now_secs: i64) -> usize {
    let pending = match acquire(&state.store).await {
        Ok(mut conn) => pending_time_up_in(&mut conn).await,
        Err(e) => Err(e),
    };
    let pending = match pending {
        Ok(pending) => pending,
        Err(error) => {
            tracing::error!(%error, "cannot read the matches pending time-up");
            return 0;
        }
    };
    let mut applied = 0;
    for match_id in pending {
        match time_up_match(state, &match_id, now_secs).await {
            Ok(true) => applied += 1,
            Ok(false) => {}
            Err(error) => tracing::error!(match_id, %error, "time-up failed"),
        }
    }
    applied
}

async fn acquire(store: &Store) -> Result<sqlx::pool::PoolConnection<sqlx::Sqlite>, StoreError> {
    Ok(store.pool().acquire().await?)
}

/// One runner pass as of `now_ms`: the go of every due slot (practice points cleared), then
/// time-up (due slots marked done, then every flagged match of a slot that has gone). Returns
/// how many time-ups it applied. Failures are logged; whatever they left undone is retried on
/// the next pass.
pub async fn fire_due(state: &AppState, now_ms: i64) -> usize {
    let now_secs = now();
    fire_goes(state, now_ms, now_secs).await;
    close_due_slots(state, now_ms).await;
    fire_time_ups(state, now_secs).await
}

/// How long to wait for the next pass: until the next deadline, capped at [`RECHECK`] while a
/// non-stop event is running; `None` (wake only) when none is.
async fn next_wait(store: &Store, now_ms: i64) -> Result<Option<Duration>, StoreError> {
    let mut conn = acquire(store).await?;
    if !any_running_non_stop_in(&mut conn).await? {
        return Ok(None);
    }
    let wait = match next_deadline_in(&mut conn).await? {
        Some(at) => {
            let ms = u64::try_from(at.saturating_sub(now_ms).max(0)).unwrap_or(0);
            Duration::from_millis(ms).min(RECHECK)
        }
        None => RECHECK,
    };
    Ok(Some(wait.max(MIN_SLEEP)))
}

/// The clock runner: a pass at once, then one at every deadline, every [`RECHECK`] while a
/// non-stop event is running, and on every `clock_wake`, until shutdown.
pub async fn run_clock(state: AppState) {
    loop {
        let now = now_ms();
        let applied = fire_due(&state, now).await;
        if applied > 0 {
            tracing::info!(applied, "non-stop time called");
        }
        let wait = match next_wait(&state.store, now_ms()).await {
            Ok(wait) => wait,
            Err(error) => {
                tracing::error!(%error, "cannot read the next non-stop deadline");
                Some(RECHECK)
            }
        };
        let sleep = async {
            match wait {
                Some(wait) => tokio::time::sleep(wait).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            () = state.shutdown.cancelled() => return,
            () = state.clock_wake.notified() => {}
            () = sleep => {}
        }
    }
}
