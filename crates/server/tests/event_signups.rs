//! Event sign-ups: solo and partner sign-ups, automatic pairing, the line, and the join page
//! (Task 13). Store tests use an explicit clock from `T0`; the API tests the real one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use axum::Router;
use axum::http::{Method, StatusCode};
use padel_score_server::AppState;
use padel_score_server::accounts_store::NewUser;
use padel_score_server::api::now;
use padel_score_server::competitions::schedule::{ScheduleInput, Signups};
use padel_score_server::competitions::signups::{JoinAs, MyEntry, SignupList, SignupTeam};
use padel_score_server::competitions::{CompetitionChange, Kind, NewCompetition, Status};
use padel_score_server::store::StoreError;
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{Res, app_from, as_session, fast_user, req, send, state};

const T0: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

fn one_set() -> Format {
    Format::Sets(SetRules {
        deuce: DeuceRule::Advantage,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    })
}

struct World {
    state: AppState,
    pool: SqlitePool,
    app: Router,
    owner: String,
    raw: String,
    event: String,
    token: String,
}

/// A public event of `kind` with room for `max` teams, made at `clock` and starting at `start`.
/// A non-stop event is one leg with a minimum of two teams.
async fn world_of(kind: Kind, max: u32, start: i64, clock: i64) -> World {
    let (state, pool) = state().await;
    let (owner, raw) = fast_user(&state, "owner@x.test", "Owner").await;
    let (name, format, settings, min_teams) = match kind {
        Kind::Tournament => ("Friday cup", one_set(), None, None),
        Kind::NonStop => (
            "Americano",
            Format::Timed {
                deuce: DeuceRule::Advantage,
                draws_allowed: false,
            },
            Some(json!({ "legs": 1 })),
            Some(2),
        ),
    };
    let ev = state
        .store
        .create_competition(
            &owner,
            NewCompetition {
                name: name.into(),
                kind,
                format,
                settings,
                schedule: Some(ScheduleInput {
                    starts_at: Some(Some(start)),
                    signups: Some(Signups::Public),
                    max_teams: Some(Some(max)),
                    min_teams,
                    ..Default::default()
                }),
            },
            clock,
        )
        .await
        .unwrap();
    let app = app_from(&state, 1);
    World {
        state,
        pool,
        app,
        owner,
        raw,
        token: ev.join_token.unwrap(),
        event: ev.id,
    }
}

/// A public tournament with room for `max` teams, made at `clock` and starting at `start`.
async fn world_with(max: u32, start: i64, clock: i64) -> World {
    world_of(Kind::Tournament, max, start, clock).await
}

/// Starts in three days, so the default deadline is `T0 + 2 * DAY`.
async fn world(max: u32) -> World {
    world_with(max, T0 + 3 * DAY, T0).await
}

/// The same, on the real clock.
async fn live_world() -> World {
    world_with(4, now() + 3 * DAY, now()).await
}

impl World {
    /// A new user named `name` (store only, no session).
    async fn user(&self, name: &str) -> String {
        self.state
            .store
            .create_user(
                NewUser {
                    email: format!("{}@x.test", name.to_lowercase()),
                    display_name: name.to_owned(),
                    password_hash: "x".to_owned(),
                },
                T0,
            )
            .await
            .unwrap()
            .id
    }

    async fn join(&self, user: &str, as_: JoinAs, at: i64) -> Result<MyEntry, StoreError> {
        self.state
            .store
            .join_event(&self.token, user, as_, at)
            .await
    }

    async fn solo(&self, user: &str, at: i64) -> MyEntry {
        self.join(user, JoinAs::Solo, at).await.unwrap()
    }

    /// A complete team of `user` and a partner named `name`.
    async fn named(&self, user: &str, name: &str, at: i64) -> MyEntry {
        let as_ = JoinAs::Partner {
            partner_name: Some(name.to_owned()),
        };
        self.join(user, as_, at).await.unwrap()
    }

    /// A partner link for `user`: its token.
    async fn invite(&self, user: &str, at: i64) -> String {
        let as_ = JoinAs::Partner { partner_name: None };
        match self.join(user, as_, at).await.unwrap() {
            MyEntry::Pending { partner_token } => partner_token,
            other => panic!("a link, not {other:?}"),
        }
    }

    async fn leave(&self, user: &str, at: i64) -> Result<(), StoreError> {
        self.state.store.leave_event(&self.token, user, at).await
    }

    async fn list(&self) -> SignupList {
        self.state
            .store
            .signup_list(&self.owner, &self.event)
            .await
            .unwrap()
            .unwrap()
    }

    async fn me(&self, user: &str) -> Option<MyEntry> {
        self.state
            .store
            .join_page(&self.token, Some(user), T0)
            .await
            .unwrap()
            .unwrap()
            .me
    }

    /// The notices queued for `user`, oldest first.
    async fn notices(&self, user: &str) -> Vec<Value> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT payload FROM push_outbox WHERE user_id = ? ORDER BY rowid")
                .bind(user)
                .fetch_all(&self.pool)
                .await
                .unwrap();
        rows.iter()
            .map(|p| serde_json::from_str(p).unwrap())
            .collect()
    }

    async fn kinds(&self, user: &str) -> Vec<String> {
        self.notices(user)
            .await
            .iter()
            .map(|n| n["kind"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM event_signups WHERE event_id = ?")
            .bind(&self.event)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn user1_at(&self, user: &str) -> i64 {
        sqlx::query_scalar("SELECT user1_at FROM event_signups WHERE user1_id = ?")
            .bind(user)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn call(&self, raw: &str, method: Method, path: &str, body: Option<Value>) -> Res {
        send(&self.app, as_session(&self.state, raw, method, path, body)).await
    }

    async fn anon(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        send(&self.app, req(method, path, body, &[])).await
    }
}

fn names(team: &SignupTeam) -> Vec<&str> {
    team.players.iter().map(|p| p.name.as_str()).collect()
}

#[tokio::test]
async fn solo_then_solo_pairs_automatically() {
    let w = world(4).await;
    let (a, b) = (w.user("Ana").await, w.user("Rui").await);
    // A pair that is complete earlier takes the first place.
    let early = w.user("Marta").await;
    w.named(&early, "Joana", T0 + 5).await;
    assert_eq!(w.solo(&a, T0 + 10).await, MyEntry::Solo);
    assert!(w.notices(&a).await.is_empty());
    // The place counts from the second sign-up.
    assert_eq!(w.solo(&b, T0 + 20).await, MyEntry::Team { place: 2 });
    let list = w.list().await;
    assert!(list.solos.is_empty());
    assert_eq!(list.teams.len(), 2);
    let team = &list.teams[1];
    assert_eq!(names(team), ["Ana", "Rui"]);
    assert_eq!(team.complete_at, Some(T0 + 20));
    assert!(team.auto_paired && !team.held);
    for (user, partner) in [(&a, "Rui"), (&b, "Ana")] {
        let notices = w.notices(user).await;
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0]["kind"], "team_complete");
        assert_eq!(notices[0]["name"], partner);
        assert_eq!(notices[0]["url"], format!("/j/{}", w.token));
    }
    assert_eq!(w.rows().await, 2);
}

#[tokio::test]
async fn pair_joining_later_goes_ahead_of_a_waiting_solo() {
    let w = world(4).await;
    let (x, y, a) = (
        w.user("Xavi").await,
        w.user("Yara").await,
        w.user("Ana").await,
    );
    assert_eq!(w.solo(&x, T0 + 10).await, MyEntry::Solo);
    // Ana joins later with a named partner: complete at once, so she has a place.
    assert_eq!(
        w.named(&a, "Bia", T0 + 20).await,
        MyEntry::Team { place: 1 }
    );
    // Yara pairs with the waiting Xavi, and that team is complete after Ana's.
    assert_eq!(w.solo(&y, T0 + 30).await, MyEntry::Team { place: 2 });
    let list = w.list().await;
    assert_eq!(names(&list.teams[0]), ["Ana", "Bia"]);
    assert_eq!(names(&list.teams[1]), ["Xavi", "Yara"]);
}

#[tokio::test]
async fn partner_link_completes_on_accept() {
    let w = world(4).await;
    let s = &w.state.store;
    let (a, b) = (w.user("Ana").await, w.user("Rui").await);
    let ptoken = w.invite(&a, T0 + 10).await;
    let list = w.list().await;
    assert_eq!(list.pending.len(), 1);
    assert!(list.pending[0].partner_pending);
    assert!(list.teams.is_empty() && list.solos.is_empty());
    // The invitation page.
    let invite = s.partner_invite(&ptoken).await.unwrap().unwrap();
    assert_eq!(invite.event_name, "Friday cup");
    assert_eq!(invite.inviter_name, "Ana");
    assert_eq!(invite.join_token, w.token);
    assert!(s.partner_invite("nope").await.unwrap().is_none());
    // Accepting completes the team now.
    let mine = s.accept_partner(&ptoken, &b, T0 + 50).await.unwrap();
    assert_eq!(mine, MyEntry::Team { place: 1 });
    let list = w.list().await;
    assert!(list.pending.is_empty());
    assert_eq!(list.teams[0].complete_at, Some(T0 + 50));
    assert!(!list.teams[0].auto_paired);
    assert_eq!(w.kinds(&a).await, ["team_complete"]);
    assert_eq!(w.kinds(&b).await, ["team_complete"]);
    // The link is spent.
    let again = s.accept_partner(&ptoken, &b, T0 + 60).await;
    assert!(matches!(again, Err(StoreError::NotFound)));
    assert!(s.partner_invite(&ptoken).await.unwrap().is_none());
}

#[tokio::test]
async fn declining_a_partner_link_gives_the_inviter_a_fresh_one() {
    let w = world(4).await;
    let s = &w.state.store;
    let (a, b) = (w.user("Ana").await, w.user("Rui").await);
    let old = w.invite(&a, T0 + 10).await;
    s.decline_partner(&old, &b).await.unwrap();
    // The old link is dead; the inviter is still pending, now with another link.
    assert!(s.partner_invite(&old).await.unwrap().is_none());
    let Some(MyEntry::Pending { partner_token: new }) = w.me(&a).await else {
        panic!("still pending");
    };
    assert_ne!(new, old);
    assert!(s.partner_invite(&new).await.unwrap().is_some());
    assert!(matches!(
        s.decline_partner(&old, &b).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn partner_by_name_completes_at_once() {
    let w = world(4).await;
    let (a, b, c) = (
        w.user("Ana").await,
        w.user("Rui").await,
        w.user("Sam").await,
    );
    assert_eq!(
        w.named(&a, "  Marta Costa ", T0 + 10).await,
        MyEntry::Team { place: 1 }
    );
    let list = w.list().await;
    assert_eq!(list.teams[0].players[1].name, "Marta Costa");
    assert_eq!(list.teams[0].players[1].user_id, None);
    assert_eq!(list.teams[0].complete_at, Some(T0 + 10));
    // Only a trimmed name of 1 to 40 characters.
    for bad in ["   ", &"x".repeat(41)] {
        let as_ = JoinAs::Partner {
            partner_name: Some(bad.to_owned()),
        };
        let res = w.join(&b, as_, T0 + 20).await;
        assert!(
            matches!(res, Err(StoreError::InvalidField("partner_name"))),
            "{bad:?}"
        );
    }
    assert_eq!(
        w.named(&c, &"x".repeat(40), T0 + 30).await,
        MyEntry::Team { place: 2 }
    );
    assert_eq!(w.rows().await, 2);
}

#[tokio::test]
async fn cannot_accept_own_partner_link() {
    let w = world(4).await;
    let a = w.user("Ana").await;
    let ptoken = w.invite(&a, T0 + 10).await;
    let own = w.state.store.accept_partner(&ptoken, &a, T0 + 20).await;
    assert!(matches!(own, Err(StoreError::Conflict("own_link"))));
    // Nothing changed: still one pending row and no notice.
    assert_eq!(w.rows().await, 1);
    assert_eq!(w.list().await.pending.len(), 1);
    assert!(w.notices(&a).await.is_empty());
}

#[tokio::test]
async fn user_cannot_sign_up_twice() {
    let w = world(4).await;
    let (a, b, c) = (
        w.user("Ana").await,
        w.user("Rui").await,
        w.user("Sam").await,
    );
    w.solo(&a, T0 + 10).await;
    let twice = [
        JoinAs::Solo,
        JoinAs::Partner { partner_name: None },
        JoinAs::Partner {
            partner_name: Some("Bia".into()),
        },
    ];
    for as_ in twice {
        let res = w.join(&a, as_, T0 + 20).await;
        assert!(
            matches!(res, Err(StoreError::Conflict("already_signed_up"))),
            "{res:?}"
        );
    }
    // The second member of a pair is signed up too.
    w.solo(&b, T0 + 30).await;
    let res = w.join(&b, JoinAs::Solo, T0 + 40).await;
    assert!(matches!(
        res,
        Err(StoreError::Conflict("already_signed_up"))
    ));
    // Accepting a link while already in the event is refused too.
    let ptoken = w.invite(&c, T0 + 50).await;
    let res = w.state.store.accept_partner(&ptoken, &a, T0 + 60).await;
    assert!(matches!(
        res,
        Err(StoreError::Conflict("already_signed_up"))
    ));
    assert_eq!(w.rows().await, 2);
}

#[tokio::test]
async fn leaving_sends_partner_back_to_solos_with_their_own_time() {
    let w = world(4).await;
    let (a, b, c, d) = (
        w.user("Ana").await,
        w.user("Rui").await,
        w.user("Sam").await,
        w.user("Tea").await,
    );
    w.solo(&a, T0 + 10).await;
    w.solo(&b, T0 + 20).await;
    // The first to sign up leaves: Rui is a solo again, with his own time.
    w.leave(&a, T0 + 30).await.unwrap();
    assert_eq!(w.rows().await, 1);
    assert_eq!(w.user1_at(&b).await, T0 + 20);
    let list = w.list().await;
    assert!(list.teams.is_empty());
    assert_eq!(names(&list.solos[0]), ["Rui"]);
    let notices = w.notices(&b).await;
    assert_eq!(notices.last().unwrap()["kind"], "partner_left");
    assert_eq!(notices.last().unwrap()["name"], "Ana");
    // The leaver is told nothing about her own action.
    assert_eq!(w.kinds(&a).await, ["team_complete"]);
    // Rui pairs again with the next solo.
    assert_eq!(w.solo(&c, T0 + 40).await, MyEntry::Team { place: 1 });
    // The second member leaves: the first keeps their own time as a solo.
    w.leave(&c, T0 + 50).await.unwrap();
    assert_eq!(w.user1_at(&b).await, T0 + 20);
    assert_eq!(w.solo(&d, T0 + 60).await, MyEntry::Team { place: 1 });
    // A solo, and a team with a named partner, leave with their row.
    w.leave(&d, T0 + 70).await.unwrap();
    w.leave(&b, T0 + 71).await.unwrap();
    assert_eq!(w.rows().await, 0);
    w.named(&a, "Bia", T0 + 80).await;
    w.leave(&a, T0 + 90).await.unwrap();
    assert_eq!(w.rows().await, 0);
    assert!(matches!(
        w.leave(&a, T0 + 100).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn sign_ups_in_one_second_keep_their_arrival_order() {
    let w = world(4).await;
    // Arrive in descending id order, all in the same second: an id tie-break would reverse them.
    let mut users = Vec::new();
    for n in ["T1", "T2", "T3", "T4", "T5", "T6"] {
        users.push((w.user(n).await, n));
    }
    users.sort_by(|a, b| b.0.cmp(&a.0));
    for (id, n) in &users {
        w.named(id, &format!("P{n}"), T0 + 5).await;
    }
    let list = w.list().await;
    let first = |t: &SignupTeam| names(t)[0].to_owned();
    let teams: Vec<_> = list.teams.iter().map(first).collect();
    let line: Vec<_> = list.line.iter().map(first).collect();
    let arrived: Vec<_> = users.iter().map(|(_, n)| (*n).to_owned()).collect();
    assert_eq!(teams, arrived[..4]);
    assert_eq!(line, arrived[4..]);
}

#[tokio::test]
async fn solos_in_one_second_pair_in_arrival_order() {
    let w = world(8).await;
    let mut users = Vec::new();
    for n in ["S1", "S2", "S3", "S4"] {
        users.push((w.user(n).await, n));
    }
    users.sort_by(|a, b| b.0.cmp(&a.0));
    for (id, _) in &users {
        w.solo(id, T0 + 5).await;
    }
    let list = w.list().await;
    let pairs: Vec<Vec<String>> = list
        .teams
        .iter()
        .map(|t| names(t).iter().map(|n| (*n).to_owned()).collect())
        .collect();
    let n = |i: usize| users[i].1.to_owned();
    assert_eq!(pairs, [vec![n(0), n(1)], vec![n(2), n(3)]]);
}

#[tokio::test]
async fn line_moves_up_when_a_team_leaves() {
    let w = world(4).await;
    let mut users = Vec::new();
    for (i, n) in ["U1", "U2", "U3", "U4", "U5"].iter().enumerate() {
        let u = w.user(n).await;
        let at = T0 + 10 * i64::try_from(i + 1).unwrap();
        let entry = w.named(&u, &format!("P{i}"), at).await;
        if i < 4 {
            let place = u32::try_from(i + 1).unwrap();
            assert_eq!(entry, MyEntry::Team { place });
        } else {
            assert_eq!(entry, MyEntry::Line { place: 1 });
        }
        users.push(u);
    }
    let list = w.list().await;
    assert_eq!(list.teams.len(), 4);
    assert_eq!(list.line.len(), 1);
    assert_eq!(names(&list.line[0]), ["U5", "P4"]);
    // The fifth team in the line hears nothing yet.
    assert!(w.notices(&users[4]).await.is_empty());
    // The second team leaves: the line moves up, and only the one who moved is told.
    w.leave(&users[1], T0 + 100).await.unwrap();
    assert_eq!(w.kinds(&users[4]).await, ["moved_up"]);
    for u in &users[..4] {
        assert!(w.notices(u).await.is_empty(), "nobody else");
    }
    let list = w.list().await;
    assert_eq!(list.teams.len(), 4);
    assert!(list.line.is_empty());
    assert_eq!(w.me(&users[4]).await, Some(MyEntry::Team { place: 4 }));
}

#[tokio::test]
async fn hand_added_team_pushes_last_signup_to_the_line() {
    let w = world(4).await;
    let s = &w.state.store;
    let mut users = Vec::new();
    for i in 0..4 {
        let u = w.user(&format!("U{i}")).await;
        w.named(&u, &format!("P{i}"), T0 + 10 + i).await;
        users.push(u);
    }
    let a = s
        .create_or_get_player(&w.owner, "Hand A", T0)
        .await
        .unwrap()
        .0
        .id;
    let b = s
        .create_or_get_player(&w.owner, "Hand B", T0)
        .await
        .unwrap()
        .0
        .id;
    let team = s
        .add_team(&w.owner, &w.event, [a, b], None, T0 + 100)
        .await
        .unwrap();
    // The hand-added team has a guaranteed place, so the last sign-up drops to the line.
    assert_eq!(w.kinds(&users[3]).await, ["moved_down"]);
    for u in &users[..3] {
        assert!(w.notices(u).await.is_empty());
    }
    let list = w.list().await;
    assert_eq!(list.hand_added, 1);
    assert_eq!(list.teams.len(), 3);
    assert_eq!(list.line.len(), 1);
    // The places count the hand-added team first.
    assert_eq!(w.me(&users[0]).await, Some(MyEntry::Team { place: 2 }));
    // Taking it away brings the last sign-up back.
    s.delete_team(&w.owner, &w.event, &team.id, T0 + 200)
        .await
        .unwrap();
    assert_eq!(w.kinds(&users[3]).await, ["moved_down", "moved_up"]);
    assert_eq!(w.list().await.hand_added, 0);
}

#[tokio::test]
async fn signups_closed_after_deadline() {
    let w = world(4).await;
    let deadline = T0 + 2 * DAY; // 24h before the start, which is three days out
    let (a, b, c, d, e) = (
        w.user("Ana").await,
        w.user("Rui").await,
        w.user("Sam").await,
        w.user("Tea").await,
        w.user("Uma").await,
    );
    w.solo(&a, deadline - 1).await;
    let late = w.join(&b, JoinAs::Solo, deadline).await;
    assert!(matches!(late, Err(StoreError::Conflict("signups_closed"))));
    let ptoken = w.invite(&c, deadline - 1).await;
    // A partner link is closed with the sign-ups too.
    let res = w.state.store.accept_partner(&ptoken, &d, deadline).await;
    assert!(matches!(res, Err(StoreError::Conflict("signups_closed"))));
    // Leaving stays open after the deadline.
    w.leave(&a, deadline + 10).await.unwrap();
    // Closing sign-ups early (taking the teams) closes them too.
    sqlx::query("UPDATE competitions SET signups_closed_at = ? WHERE id = ?")
        .bind(T0 + 5)
        .bind(&w.event)
        .execute(&w.pool)
        .await
        .unwrap();
    let res = w.join(&e, JoinAs::Solo, T0 + 6).await;
    assert!(matches!(res, Err(StoreError::Conflict("signups_closed"))));
}

#[tokio::test]
async fn cancelled_or_unknown_events_refuse_sign_ups() {
    let w = world(4).await;
    let (a, b) = (w.user("Ana").await, w.user("Rui").await);
    w.solo(&a, T0 + 10).await;
    w.state
        .store
        .cancel_competition(&w.owner, &w.event, T0 + 20)
        .await
        .unwrap();
    let res = w.join(&b, JoinAs::Solo, T0 + 30).await;
    assert!(matches!(res, Err(StoreError::Conflict("event_cancelled"))));
    let res = w.leave(&a, T0 + 30).await;
    assert!(matches!(res, Err(StoreError::Conflict("event_cancelled"))));
    let list = w.state.store.signup_list(&w.owner, &w.event).await;
    assert!(list.unwrap().is_some(), "reads stay open");
    let res = w.state.store.join_event("nope", &b, JoinAs::Solo, T0).await;
    assert!(matches!(res, Err(StoreError::NotFound)));
    let page = w.state.store.join_page("nope", None, T0).await;
    assert!(page.unwrap().is_none());
}

#[tokio::test]
async fn signups_are_capped_per_event() {
    let w = world(4).await;
    let a = w.user("Ana").await;
    for n in 0..256 {
        sqlx::query(
            "INSERT INTO event_signups (id, event_id, user1_id, user1_at, held) \
             VALUES (?, ?, ?, ?, 1)",
        )
        .bind(format!("row-{n}"))
        .bind(&w.event)
        .bind(&w.owner)
        .bind(T0 + n)
        .execute(&w.pool)
        .await
        .unwrap();
    }
    let res = w.join(&a, JoinAs::Solo, T0 + 1000).await;
    assert!(matches!(res, Err(StoreError::Conflict("signups_full"))));
}

#[tokio::test]
async fn organiser_split_and_pair() {
    let w = world(4).await;
    let s = &w.state.store;
    let (a, b, c, e) = (
        w.user("Ana").await,
        w.user("Rui").await,
        w.user("Sam").await,
        w.user("Uma").await,
    );
    w.solo(&a, T0 + 10).await;
    w.solo(&b, T0 + 20).await;
    w.named(&e, "Bia", T0 + 25).await;
    let list = w.list().await;
    let auto = list
        .teams
        .iter()
        .find(|t| t.auto_paired)
        .unwrap()
        .id
        .clone();
    let named = list
        .teams
        .iter()
        .find(|t| !t.auto_paired)
        .unwrap()
        .id
        .clone();
    // Only an automatic pair can be split.
    let res = s.split_signup(&w.owner, &w.event, &named, T0 + 30).await;
    assert!(matches!(res, Err(StoreError::Conflict("not_auto_paired"))));
    s.split_signup(&w.owner, &w.event, &auto, T0 + 30)
        .await
        .unwrap();
    let list = w.list().await;
    assert_eq!(list.teams.len(), 1);
    assert_eq!(list.solos.len(), 2);
    assert!(list.solos.iter().all(|t| t.held));
    assert_eq!(names(&list.solos[0]), ["Ana"]);
    assert_eq!(names(&list.solos[1]), ["Rui"]);
    // Held solos show as solos to themselves, and auto pairing skips them.
    assert_eq!(w.me(&a).await, Some(MyEntry::Solo));
    assert_eq!(w.solo(&c, T0 + 40).await, MyEntry::Solo);
    assert_eq!(w.list().await.solos.len(), 3);
    // The organiser pairs two solos by hand: the team is complete now.
    let l = w.list().await;
    let find = |n: &str| l.solos.iter().find(|t| names(t) == [n]).unwrap().id.clone();
    let (a_id, c_id, b_id) = (find("Ana"), find("Sam"), find("Rui"));
    s.pair_signups(&w.owner, &w.event, &a_id, &c_id, T0 + 50)
        .await
        .unwrap();
    let list = w.list().await;
    assert_eq!(list.teams.len(), 2);
    assert_eq!(list.teams[1].complete_at, Some(T0 + 50));
    assert_eq!(names(&list.teams[1]), ["Ana", "Sam"]);
    assert!(!list.teams[1].auto_paired && !list.teams[1].held);
    assert_eq!(list.solos.len(), 1);
    // Both get team_complete (the organiser is neither).
    assert_eq!(w.kinds(&c).await, ["team_complete"]);
    assert_eq!(w.kinds(&a).await, ["team_complete", "team_complete"]);
    // Pairing a non-solo, or an unknown entry, is refused.
    let res = s
        .pair_signups(&w.owner, &w.event, &b_id, &named, T0 + 60)
        .await;
    assert!(matches!(res, Err(StoreError::Conflict("not_solo"))));
    let res = s
        .pair_signups(&w.owner, &w.event, &b_id, "nope", T0 + 60)
        .await;
    assert!(matches!(res, Err(StoreError::NotFound)));
    let res = s
        .pair_signups(&w.owner, &w.event, &b_id, &b_id, T0 + 60)
        .await;
    assert!(matches!(res, Err(StoreError::InvalidField("entries"))));
    // Removing a solo and a team.
    s.remove_signup(&w.owner, &w.event, &b_id, T0 + 70)
        .await
        .unwrap();
    s.remove_signup(&w.owner, &w.event, &named, T0 + 70)
        .await
        .unwrap();
    assert_eq!(w.rows().await, 1);
    // A team the organiser took cannot be removed, and cannot leave.
    let left = w.list().await.teams[0].id.clone();
    let p1 = s
        .create_or_get_player(&w.owner, "P1", T0)
        .await
        .unwrap()
        .0
        .id;
    let p2 = s
        .create_or_get_player(&w.owner, "P2", T0)
        .await
        .unwrap()
        .0
        .id;
    let team = s
        .add_team(&w.owner, &w.event, [p1, p2], None, T0 + 80)
        .await
        .unwrap();
    sqlx::query("UPDATE event_signups SET taken_team_id = ? WHERE id = ?")
        .bind(&team.id)
        .bind(&left)
        .execute(&w.pool)
        .await
        .unwrap();
    let res = s.remove_signup(&w.owner, &w.event, &left, T0 + 90).await;
    assert!(matches!(res, Err(StoreError::Conflict("signup_taken"))));
    let res = w.leave(&a, T0 + 90).await;
    assert!(matches!(res, Err(StoreError::Conflict("signup_taken"))));
}

#[tokio::test]
async fn another_owner_cannot_see_the_list() {
    let w = world(4).await;
    let s = &w.state.store;
    let a = w.user("Ana").await;
    w.named(&a, "Bia", T0 + 10).await;
    let other = w.user("Other").await;
    assert!(s.signup_list(&other, &w.event).await.unwrap().is_none());
    let entry = w.list().await.teams[0].id.clone();
    let res = s.remove_signup(&other, &w.event, &entry, T0 + 20).await;
    assert!(matches!(res, Err(StoreError::NotFound)));
    let res = s.split_signup(&other, &w.event, &entry, T0 + 20).await;
    assert!(matches!(res, Err(StoreError::NotFound)));
    let res = s.pair_signups(&other, &w.event, &entry, "x", T0 + 20).await;
    assert!(matches!(res, Err(StoreError::NotFound)));
    assert_eq!(w.rows().await, 1);
}

#[tokio::test]
async fn schedule_change_tells_signed_up_users() {
    let w = world(4).await;
    let a = w.user("Ana").await;
    w.named(&a, "Bia", T0 + 10).await;
    let change = CompetitionChange {
        schedule: Some(ScheduleInput {
            place: Some(Some("Court 9".into())),
            ..Default::default()
        }),
        ..Default::default()
    };
    w.state
        .store
        .update_competition(&w.owner, &w.event, change, T0 + 20)
        .await
        .unwrap();
    let notices = w.notices(&a).await;
    assert_eq!(notices.last().unwrap()["kind"], "event_changed");
    assert_eq!(notices.last().unwrap()["url"], format!("/j/{}", w.token));
}

// ----- HTTP -----

#[tokio::test]
async fn signed_out_reads_join_page_with_no_me() {
    let w = live_world().await;
    let a = w.user("Ana").await;
    w.named(&a, "Bia", now()).await;
    let path = format!("/api/join/{}", w.token);
    let res = w.anon(Method::GET, &path, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["kind"], "event");
    assert_eq!(res.body["event"]["name"], "Friday cup");
    assert_eq!(res.body["event"]["kind"], "tournament");
    assert_eq!(res.body["event"]["organiser_name"], "Owner");
    assert_eq!(res.body["event"]["closed"], false);
    assert!(res.body["event"]["schedule"]["starts_at"].is_i64());
    assert!(res.body["me"].is_null());
    assert_eq!(res.body["list"]["teams"].as_array().unwrap().len(), 1);
    // The page is a secret: never cached, and no account ids leak.
    assert_eq!(
        res.header(axum::http::header::CACHE_CONTROL),
        Some("no-store")
    );
    assert!(!res.body.to_string().contains(&a));
    // Joining needs a session.
    let res = w
        .anon(Method::POST, &path, Some(json!({ "as": "solo" })))
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let res = w.anon(Method::GET, "/api/join/nope", None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_event_needs_the_link_public_is_listed() {
    let w = live_world().await;
    let path = format!("/api/join/{}", w.token);
    let event = format!("/api/events/{}", w.event);
    // A private event reads by its link like a public one.
    let change = json!({ "schedule": { "signups": "private" } });
    let res = w.call(&w.raw, Method::PATCH, &event, Some(change)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = w.anon(Method::GET, &path, None).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["event"]["name"], "Friday cup");
    // Turning sign-ups off hides the page again.
    let change = json!({ "schedule": { "signups": "off" } });
    let res = w.call(&w.raw, Method::PATCH, &event, Some(change)).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let res = w.anon(Method::GET, &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn join_leave_and_partner_flow_over_http() {
    let w = live_world().await;
    let (_, a_raw) = fast_user(&w.state, "ana@y.test", "Ana").await;
    let (_, b_raw) = fast_user(&w.state, "rui@y.test", "Rui").await;
    let join = format!("/api/join/{}", w.token);
    let res = w
        .call(&a_raw, Method::POST, &join, Some(json!({ "as": "solo" })))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!({ "state": "solo" }));
    // Twice is a 409, never a second row.
    for body in [
        json!({ "as": "solo" }),
        json!({ "as": "partner" }),
        json!({ "as": "partner", "partner_name": "Bia" }),
    ] {
        let res = w.call(&a_raw, Method::POST, &join, Some(body)).await;
        assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
        assert_eq!(res.body["error"], "already_signed_up");
    }
    assert_eq!(w.rows().await, 1);
    // The join page tells Ana where she is.
    let res = w.call(&a_raw, Method::GET, &join, None).await;
    assert_eq!(res.body["me"], json!({ "state": "solo" }));
    // Leave is 204; leaving again is 404.
    let res = w.call(&a_raw, Method::DELETE, &join, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = w.call(&a_raw, Method::DELETE, &join, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // A partner link, its page, the inviter's own link, and the accept.
    let res = w
        .call(
            &a_raw,
            Method::POST,
            &join,
            Some(json!({ "as": "partner" })),
        )
        .await;
    assert_eq!(res.body["state"], "pending");
    let ptoken = res.body["partner_token"].as_str().unwrap().to_owned();
    let page = format!("/api/partner/{ptoken}");
    let res = w.anon(Method::GET, &page, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["event_name"], "Friday cup");
    assert_eq!(res.body["inviter_name"], "Ana");
    assert_eq!(res.body["join_token"], w.token);
    assert_eq!(
        res.header(axum::http::header::CACHE_CONTROL),
        Some("no-store")
    );
    let accept = format!("{page}/accept");
    let res = w.call(&a_raw, Method::POST, &accept, None).await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.body);
    assert_eq!(res.body["error"], "own_link");
    let res = w.anon(Method::POST, &accept, None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    let res = w.call(&b_raw, Method::POST, &accept, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body, json!({ "state": "team", "place": 1 }));
    let res = w.call(&b_raw, Method::POST, &accept, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // Decline: 204 and a fresh link for the inviter.
    let (_, c_raw) = fast_user(&w.state, "sam@y.test", "Sam").await;
    let (_, d_raw) = fast_user(&w.state, "tea@y.test", "Tea").await;
    let res = w
        .call(
            &c_raw,
            Method::POST,
            &join,
            Some(json!({ "as": "partner" })),
        )
        .await;
    let ptoken = res.body["partner_token"].as_str().unwrap().to_owned();
    let decline = format!("/api/partner/{ptoken}/decline");
    let res = w.call(&d_raw, Method::POST, &decline, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let res = w.call(&c_raw, Method::GET, &join, None).await;
    assert_ne!(res.body["me"]["partner_token"], json!(ptoken));
}

#[tokio::test]
async fn organiser_routes_over_http() {
    let w = live_world().await;
    let (_, a_raw) = fast_user(&w.state, "ana@y.test", "Ana").await;
    let (_, b_raw) = fast_user(&w.state, "rui@y.test", "Rui").await;
    let (_, c_raw) = fast_user(&w.state, "sam@y.test", "Sam").await;
    let join = format!("/api/join/{}", w.token);
    for raw in [&a_raw, &b_raw] {
        let res = w
            .call(raw, Method::POST, &join, Some(json!({ "as": "solo" })))
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }
    let base = format!("/api/events/{}/signups", w.event);
    let res = w.call(&w.raw, Method::GET, &base, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["teams"].as_array().unwrap().len(), 1);
    let entry = res.body["teams"][0]["id"].as_str().unwrap().to_owned();
    // Another user: 404 on every route.
    let res = w.call(&c_raw, Method::GET, &base, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let split = format!("{base}/{entry}/split");
    let res = w.call(&c_raw, Method::POST, &split, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // Split gives the fresh list.
    let res = w.call(&w.raw, Method::POST, &split, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["solos"].as_array().unwrap().len(), 2);
    let a_id = res.body["solos"][0]["id"].as_str().unwrap().to_owned();
    let b_id = res.body["solos"][1]["id"].as_str().unwrap().to_owned();
    // Pair them again; the merged entry is gone, so a repeat is a 404.
    let pair = format!("{base}/pair");
    let body = json!({ "entries": [a_id, b_id] });
    let res = w
        .call(&w.raw, Method::POST, &pair, Some(body.clone()))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["teams"].as_array().unwrap().len(), 1);
    let res = w.call(&w.raw, Method::POST, &pair, Some(body)).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    // Remove is 204.
    let res = w.call(&w.raw, Method::GET, &base, None).await;
    let team = res.body["teams"][0]["id"].as_str().unwrap().to_owned();
    let res = w
        .call(&w.raw, Method::DELETE, &format!("{base}/{team}"), None)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(w.rows().await, 0);
}

#[tokio::test]
async fn another_owner_cannot_read_the_list_over_http() {
    let w = live_world().await;
    let (_, other_raw) = fast_user(&w.state, "other@y.test", "Other").await;
    let base = format!("/api/events/{}/signups", w.event);
    let res = w.call(&other_raw, Method::GET, &base, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = w.anon(Method::GET, &base, None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

// ---- Task 14: the deadline job and "Take teams from sign-ups" ----

/// A non-stop event with room for `max` teams, made at `T0`, starting in three days.
async fn non_stop_world(max: u32) -> World {
    world_of(Kind::NonStop, max, T0 + 3 * DAY, T0).await
}

const DEADLINE: i64 = T0 + 2 * DAY;

impl World {
    /// `n` complete teams (a user plus a partner typed by name), complete in order. Returns the
    /// users.
    async fn teams(&self, prefix: &str, n: i64) -> Vec<String> {
        let mut users = Vec::new();
        for i in 0..n {
            let u = self.user(&format!("{prefix}{i}")).await;
            self.named(&u, &format!("{prefix}Partner{i}"), T0 + 10 + i)
                .await;
            users.push(u);
        }
        users
    }

    async fn hand_team(&self, a: &str, b: &str) -> String {
        let s = &self.state.store;
        let mut ids = Vec::new();
        for name in [a, b] {
            ids.push(
                s.create_or_get_player(&self.owner, name, T0)
                    .await
                    .unwrap()
                    .0
                    .id,
            );
        }
        let [x, y]: [String; 2] = ids.try_into().unwrap();
        s.add_team(&self.owner, &self.event, [x, y], None, T0 + 5)
            .await
            .unwrap()
            .id
    }

    async fn event_now(&self) -> padel_score_server::competitions::Competition {
        self.state
            .store
            .competition(&self.owner, &self.event)
            .await
            .unwrap()
            .unwrap()
    }

    async fn team_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM competition_teams WHERE event_id = ?")
            .bind(&self.event)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn taken_rows(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM event_signups WHERE event_id = ? AND taken_team_id IS NOT NULL",
        )
        .bind(&self.event)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn take(
        &self,
        at: i64,
    ) -> Result<padel_score_server::competitions::signups::TakeReport, StoreError> {
        self.state
            .store
            .take_teams(&self.owner, &self.event, at)
            .await
    }
}

#[tokio::test]
async fn deadline_below_minimum_cancels_and_tells_everyone() {
    let w = world(16).await;
    let mut users = w.teams("T", 3).await;
    // A lone solo does not count as a team.
    let solo = w.user("Solo").await;
    w.solo(&solo, T0 + 100).await;
    users.push(solo);
    let s = &w.state.store;
    assert_eq!(s.close_due_signups(DEADLINE - 1).await.unwrap().len(), 0);
    assert_eq!(s.close_due_signups(DEADLINE).await.unwrap().len(), 1);
    let ev = w.event_now().await;
    assert_eq!(ev.status, Status::Cancelled);
    assert_eq!(ev.cancelled_at, Some(DEADLINE));
    assert_eq!(ev.signups_closed_at, Some(DEADLINE));
    for u in &users {
        assert_eq!(w.kinds(u).await, ["event_cancelled"], "{u}");
    }
    // Done once.
    assert_eq!(s.close_due_signups(DEADLINE + 60).await.unwrap().len(), 0);
}

#[tokio::test]
async fn deadline_with_enough_teams_closes_signups() {
    let w = world(16).await;
    let users = w.teams("T", 4).await;
    let s = &w.state.store;
    assert_eq!(s.close_due_signups(DEADLINE).await.unwrap().len(), 1);
    let ev = w.event_now().await;
    assert_eq!(ev.status, Status::Setup);
    assert_eq!(ev.signups_closed_at, Some(DEADLINE));
    assert!(w.kinds(&users[0]).await.is_empty());
    assert_eq!(s.close_due_signups(DEADLINE + 60).await.unwrap().len(), 0);
    let late = w.user("Late").await;
    assert!(matches!(
        w.join(&late, JoinAs::Solo, DEADLINE + 1).await,
        Err(StoreError::Conflict("signups_closed"))
    ));
}

#[tokio::test]
async fn hand_added_teams_count_toward_the_minimum() {
    let w = world(16).await;
    w.teams("T", 2).await;
    w.hand_team("Hand A", "Hand B").await;
    w.hand_team("Hand C", "Hand D").await;
    w.state.store.close_due_signups(DEADLINE).await.unwrap();
    let ev = w.event_now().await;
    assert_eq!(ev.status, Status::Setup);
    assert_eq!(ev.signups_closed_at, Some(DEADLINE));
}

#[tokio::test]
async fn deadline_job_leaves_other_events_alone() {
    // Not due yet, sign-ups off, already closed, and already running are all skipped.
    let w = world(16).await;
    let s = &w.state.store;
    assert_eq!(s.close_due_signups(DEADLINE - 1).await.unwrap().len(), 0);
    w.teams("T", 4).await;
    s.take_teams(&w.owner, &w.event, T0 + 5).await.unwrap();
    assert_eq!(s.close_due_signups(DEADLINE).await.unwrap().len(), 0);
    assert_eq!(w.event_now().await.signups_closed_at, Some(T0 + 5));
    assert_eq!(w.event_now().await.status, Status::Setup);
}

#[tokio::test]
async fn take_teams_tournament_takes_largest_bracket() {
    let w = world(16).await;
    let users = w.teams("T", 11).await;
    let report = w.take(T0 + 500).await.unwrap();
    assert_eq!(report.taken, 8);
    assert!(report.skipped.is_empty());
    assert_eq!(report.left_on_line, 3);
    assert_eq!(w.team_rows().await, 8);
    assert_eq!(w.taken_rows().await, 8);
    for u in &users[..8] {
        assert!(w.kinds(u).await.is_empty(), "{u}");
    }
    for u in &users[8..] {
        let notices = w.notices(u).await;
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["kind"], "no_place");
        assert_eq!(notices[0]["url"], format!("/j/{}", w.token));
        assert_eq!(notices[0]["title"], "Friday cup");
    }
    // A second run changes nothing.
    assert!(matches!(
        w.take(T0 + 600).await,
        Err(StoreError::Conflict("nothing_to_take"))
    ));
    assert_eq!(w.team_rows().await, 8);
    assert_eq!(w.kinds(&users[8]).await, ["no_place"]);
}

#[tokio::test]
async fn take_teams_non_stop_takes_largest_even() {
    let w = non_stop_world(10).await;
    let users = w.teams("T", 7).await;
    let report = w.take(T0 + 500).await.unwrap();
    assert_eq!((report.taken, report.left_on_line), (6, 1));
    assert_eq!(w.team_rows().await, 6);
    assert_eq!(w.kinds(&users[6]).await, ["no_place"]);
}

#[tokio::test]
async fn hand_added_teams_fill_first() {
    let w = non_stop_world(8).await;
    w.hand_team("Hand A", "Hand B").await;
    w.hand_team("Hand C", "Hand D").await;
    w.teams("T", 7).await;
    let report = w.take(T0 + 500).await.unwrap();
    assert_eq!((report.taken, report.left_on_line), (6, 1));
    assert_eq!(w.team_rows().await, 8);
    assert_eq!(w.taken_rows().await, 6);
}

#[tokio::test]
async fn taken_players_are_auto_linked_named_partner_is_plain() {
    let w = non_stop_world(8).await;
    let (ana, rui, marta) = (
        w.user("Ana").await,
        w.user("Rui").await,
        w.user("Marta").await,
    );
    w.solo(&ana, T0 + 10).await;
    w.solo(&rui, T0 + 20).await;
    w.named(&marta, "Bia", T0 + 30).await;
    w.take(T0 + 500).await.unwrap();
    let players: Vec<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT name, linked_user_id, link_auto FROM players WHERE owner_user_id = ? \
         ORDER BY name",
    )
    .bind(&w.owner)
    .fetch_all(&w.pool)
    .await
    .unwrap();
    assert_eq!(
        players,
        [
            ("Ana".to_owned(), Some(ana), true),
            ("Bia".to_owned(), None, false),
            ("Marta".to_owned(), Some(marta), true),
            ("Rui".to_owned(), Some(rui), true),
        ]
    );
    let teams = w
        .state
        .store
        .teams(&w.owner, &w.event)
        .await
        .unwrap()
        .unwrap();
    let shown: Vec<&str> = teams.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(shown, ["Ana / Rui", "Marta / Bia"]);
}

#[tokio::test]
async fn sign_up_clashing_with_a_hand_added_team_is_skipped() {
    let w = non_stop_world(8).await;
    let ana = w.user("Ana").await;
    let ana_pair = w.named(&ana, "Bia", T0 + 10).await;
    assert!(matches!(ana_pair, MyEntry::Team { .. }));
    let others = w.teams("T", 3).await;
    // The organiser has two entries for Ana's account; the hand-added team uses the older one,
    // the sign-up would use the newest.
    let s = &w.state.store;
    let old = s
        .create_or_get_player(&w.owner, "Ana Old", T0)
        .await
        .unwrap()
        .0
        .id;
    let new = s
        .create_or_get_player(&w.owner, "Ana New", T0 + 1)
        .await
        .unwrap()
        .0
        .id;
    for id in [&old, &new] {
        sqlx::query("UPDATE players SET linked_user_id = ? WHERE id = ?")
            .bind(&ana)
            .bind(id)
            .execute(&w.pool)
            .await
            .unwrap();
    }
    let other = s
        .create_or_get_player(&w.owner, "Hand B", T0)
        .await
        .unwrap()
        .0
        .id;
    s.add_team(&w.owner, &w.event, [old, other], None, T0 + 5)
        .await
        .unwrap();
    let entry: String = sqlx::query_scalar("SELECT id FROM event_signups WHERE user1_id = ?")
        .bind(&ana)
        .fetch_one(&w.pool)
        .await
        .unwrap();
    // 1 hand-added + 4 complete, max 8: four teams in all, so three come from sign-ups.
    let report = w.take(T0 + 500).await.unwrap();
    assert_eq!(report.skipped, [entry]);
    assert_eq!((report.taken, report.left_on_line), (3, 1));
    assert_eq!(w.team_rows().await, 4);
    // Ana stays on the line and is not told "no place" (her sign-up is still waiting).
    assert!(w.kinds(&ana).await.is_empty());
    for u in &others {
        assert!(w.kinds(u).await.is_empty());
    }
    assert_eq!(w.taken_rows().await, 3);
}

#[tokio::test]
async fn take_teams_closes_signups() {
    let w = world(16).await;
    w.teams("T", 4).await;
    // Before the deadline, too.
    w.take(T0 + 500).await.unwrap();
    assert_eq!(w.event_now().await.signups_closed_at, Some(T0 + 500));
    let late = w.user("Late").await;
    assert!(matches!(
        w.join(&late, JoinAs::Solo, T0 + 600).await,
        Err(StoreError::Conflict("signups_closed"))
    ));
    // The deadline job then has nothing to do.
    assert_eq!(
        w.state
            .store
            .close_due_signups(DEADLINE)
            .await
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn take_teams_needs_a_live_event_of_the_owner() {
    let w = world(16).await;
    w.teams("T", 4).await;
    let stranger = w.user("Stranger").await;
    let s = &w.state.store;
    assert!(matches!(
        s.take_teams(&stranger, &w.event, T0 + 500).await,
        Err(StoreError::NotFound)
    ));
    s.cancel_competition(&w.owner, &w.event, T0 + 500)
        .await
        .unwrap();
    assert!(matches!(
        w.take(T0 + 600).await,
        Err(StoreError::Conflict("event_cancelled"))
    ));
}

#[tokio::test]
async fn take_teams_over_http() {
    let w = world(16).await;
    w.teams("T", 5).await;
    let path = format!("/api/events/{}/take-teams", w.event);
    let res = w.call(&w.raw, Method::POST, &path, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(
        res.body,
        json!({ "taken": 4, "skipped": [], "left_on_line": 1 })
    );
    assert!(
        w.anon(Method::POST, &path, None)
            .await
            .status
            .is_client_error()
    );
    let (_, other) = fast_user(&w.state, "other@x.test", "Other").await;
    let res = w.call(&other, Method::POST, &path, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

// ---- D4 fix wave ----

#[tokio::test]
async fn taken_entry_stays_a_team_after_a_hand_added_team() {
    let w = non_stop_world(4).await;
    let users = w.teams("T", 4).await;
    assert_eq!(w.take(T0 + 500).await.unwrap().taken, 4);
    // A fifth team by hand, past the maximum: the taken sign-ups keep their places.
    w.hand_team("Hand A", "Hand B").await;
    for u in &users {
        assert!(w.kinds(u).await.is_empty(), "{u}");
    }
    assert_eq!(w.me(&users[3]).await, Some(MyEntry::Team { place: 5 }));
    let list = w.list().await;
    assert_eq!((list.teams.len(), list.line.len()), (4, 0));
    // Lowering the maximum below the taken teams does not move them either.
    let change = CompetitionChange {
        schedule: Some(ScheduleInput {
            max_teams: Some(Some(2)),
            ..Default::default()
        }),
        ..Default::default()
    };
    w.state
        .store
        .update_competition(&w.owner, &w.event, change, T0 + 600)
        .await
        .unwrap();
    for u in &users {
        assert!(w.kinds(u).await.is_empty(), "{u}");
    }
    assert!(matches!(w.me(&users[0]).await, Some(MyEntry::Team { .. })));
    assert_eq!(w.list().await.line.len(), 0);
}

#[tokio::test]
async fn take_teams_with_nothing_to_take_changes_nothing() {
    let w = world(8).await;
    let users = w.teams("T", 3).await;
    assert!(matches!(
        w.take(T0 + 500).await,
        Err(StoreError::Conflict("nothing_to_take"))
    ));
    assert_eq!(w.event_now().await.signups_closed_at, None);
    assert_eq!(w.team_rows().await, 0);
    for u in &users {
        assert!(w.kinds(u).await.is_empty());
    }
    let path = format!("/api/events/{}/take-teams", w.event);
    let res = w.call(&w.raw, Method::POST, &path, None).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body["error"], "nothing_to_take");
}

#[tokio::test]
async fn named_partner_never_reuses_a_linked_entry() {
    let w = non_stop_world(8).await;
    let (ana, rui, marta) = (
        w.user("Ana").await,
        w.user("Rui").await,
        w.user("Marta").await,
    );
    w.solo(&ana, T0 + 10).await;
    w.solo(&rui, T0 + 20).await;
    // A different Ana, without an account.
    w.named(&marta, "Ana", T0 + 30).await;
    let report = w.take(T0 + 500).await.unwrap();
    assert_eq!((report.taken, report.skipped.len()), (2, 0));
    let players: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT name, linked_user_id FROM players WHERE owner_user_id = ? ORDER BY name",
    )
    .bind(&w.owner)
    .fetch_all(&w.pool)
    .await
    .unwrap();
    assert_eq!(
        players,
        [
            ("Ana".to_owned(), Some(ana)),
            ("Ana 2".to_owned(), None),
            ("Marta".to_owned(), Some(marta)),
            ("Rui".to_owned(), Some(rui)),
        ]
    );
}

#[tokio::test]
async fn automatic_cancel_tells_the_organiser_and_reports_it() {
    let w = world(16).await;
    let users = w.teams("T", 2).await;
    let closed = w.state.store.close_due_signups(DEADLINE).await.unwrap();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].event_id, w.event);
    assert_eq!(closed[0].owner_id, w.owner);
    assert!(closed[0].cancelled);
    assert_eq!(w.kinds(&w.owner).await, ["event_cancelled"]);
    assert_eq!(w.kinds(&users[0]).await, ["event_cancelled"]);
}

#[tokio::test]
async fn closing_without_cancel_does_not_notify_the_organiser() {
    let w = world(16).await;
    w.teams("T", 4).await;
    let closed = w.state.store.close_due_signups(DEADLINE).await.unwrap();
    assert!(!closed[0].cancelled);
    assert!(w.kinds(&w.owner).await.is_empty());
}

// ---- Deferred minors: take notices, line after a take, pending inviters, decline refresh ----

#[tokio::test]
async fn take_leaves_the_rest_on_the_line_not_in_team_places() {
    let w = world(16).await;
    let users = w.teams("T", 11).await;
    w.take(T0 + 500).await.unwrap();
    let list = w.list().await;
    assert_eq!((list.teams.len(), list.line.len()), (8, 3));
    assert_eq!(w.me(&users[7]).await, Some(MyEntry::Team { place: 8 }));
    assert_eq!(w.me(&users[8]).await, Some(MyEntry::Line { place: 1 }));
    assert_eq!(w.me(&users[10]).await, Some(MyEntry::Line { place: 3 }));
}

#[tokio::test]
async fn a_later_take_tells_the_entry_that_was_left_out_it_is_in() {
    let w = non_stop_world(10).await;
    let users = w.teams("T", 7).await;
    w.take(T0 + 500).await.unwrap();
    assert_eq!(w.kinds(&users[6]).await, ["no_place"]);
    // A hand-added team makes room for the one left over.
    w.hand_team("Hand A", "Hand B").await;
    let report = w.take(T0 + 600).await.unwrap();
    assert_eq!(report.taken, 1);
    assert_eq!(w.kinds(&users[6]).await, ["no_place", "moved_up"]);
    // Entries taken the first time are not told anything.
    assert!(w.kinds(&users[0]).await.is_empty());
}

#[tokio::test]
async fn a_taken_entry_never_told_no_place_gets_no_notice() {
    let w = non_stop_world(10).await;
    let users = w.teams("T", 6).await;
    w.take(T0 + 500).await.unwrap();
    for u in &users {
        assert!(w.kinds(u).await.is_empty(), "{u}");
    }
}

#[tokio::test]
async fn the_deadline_tells_pending_inviters_they_have_no_place() {
    let w = world(16).await;
    let users = w.teams("T", 4).await;
    let inviter = w.user("Inviter").await;
    w.invite(&inviter, T0 + 20).await;
    let s = &w.state.store;
    assert_eq!(s.close_due_signups(DEADLINE).await.unwrap().len(), 1);
    assert_eq!(w.kinds(&inviter).await, ["no_place"]);
    let notices = w.notices(&inviter).await;
    assert_eq!(notices[0]["url"], format!("/j/{}", w.token));
    assert!(w.kinds(&users[0]).await.is_empty());
    // Once only, whatever tells them again.
    s.close_due_signups(DEADLINE + 60).await.unwrap();
    assert_eq!(w.kinds(&inviter).await, ["no_place"]);
}

#[tokio::test]
async fn a_cancelled_deadline_does_not_add_no_place_for_inviters() {
    let w = world(16).await;
    let inviter = w.user("Inviter").await;
    w.invite(&inviter, T0 + 20).await;
    w.state.store.close_due_signups(DEADLINE).await.unwrap();
    assert_eq!(w.event_now().await.status, Status::Cancelled);
    assert_eq!(w.kinds(&inviter).await, ["event_cancelled"]);
}

#[tokio::test]
async fn declining_a_partner_link_tells_the_events_stream() {
    let w = live_world().await;
    let (_, c_raw) = fast_user(&w.state, "sam@y.test", "Sam").await;
    let (_, d_raw) = fast_user(&w.state, "tea@y.test", "Tea").await;
    let join = format!("/api/join/{}", w.token);
    let body = Some(json!({ "as": "partner" }));
    let res = w.call(&c_raw, Method::POST, &join, body).await;
    let ptoken = res.body["partner_token"].as_str().unwrap().to_owned();
    let mut rx = w.state.hub.subscribe_event(&w.event);
    let decline = format!("/api/partner/{ptoken}/decline");
    let res = w.call(&d_raw, Method::POST, &decline, None).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert!(rx.try_recv().is_ok(), "the event stream was not told");
}
