//! A knockout tournament run end to end over HTTP only (phase 7's "done when"): the
//! coordinator sets up the event, the court phones score the matches, the coordinator corrects
//! results, and the spectator link shows the bracket and the final places.
//!
//! The draw is random, so nothing here assumes who plays whom: teams are read from the bracket
//! and winners are chosen by side (A or B), not by team.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod support;

use std::collections::HashSet;

use axum::Router;
use axum::http::{Method, StatusCode};
use padel_score_server::AppState;
use padel_scoring::{DecidingSet, DeuceRule, Format, SetRules, SetsFormat};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use support::{Res, as_session, req, send, with_admin};

fn one_set() -> Value {
    serde_json::to_value(Format::Sets(SetRules {
        deuce: DeuceRule::GoldenPoint,
        sets: SetsFormat::One,
        deciding: DecidingSet::FullSet,
    }))
    .unwrap()
}

/// How a match is scored.
#[derive(Clone, Copy, Debug)]
enum Mode {
    /// Point by point on the court phone.
    Points,
    /// A typed final score on the court phone.
    CourtFinal,
    /// The coordinator's typed result.
    Coordinator,
}

const MODES: [Mode; 3] = [Mode::Points, Mode::CourtFinal, Mode::Coordinator];

/// The coordinator's session plus the event under test.
struct Ctx {
    state: AppState,
    app: Router,
    raw: String,
    id: String,
    /// Typed results and points need a fresh client id each.
    calls: usize,
}

impl Ctx {
    async fn api(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        send(
            &self.app,
            as_session(&self.state, &self.raw, method, path, body),
        )
        .await
    }

    async fn post(&self, path: &str, body: Option<Value>) -> Res {
        self.api(Method::POST, path, body).await
    }

    /// An unauthenticated request: the court phone and the spectator link carry their own token.
    async fn anon(&self, method: Method, path: &str, body: Option<Value>) -> Res {
        send(&self.app, req(method, path, body, &[])).await
    }

    fn client_id(&mut self) -> String {
        self.calls += 1;
        format!("e2e-{}", self.calls)
    }

    /// The coordinator's event detail, after checking the invariants every state must keep.
    async fn detail(&self) -> Value {
        let res = self
            .api(Method::GET, &format!("/api/events/{}", self.id), None)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        check_invariants(&res.body);
        res.body
    }

    async fn court_state(&self, scorer: &str) -> Value {
        let res = self
            .anon(Method::GET, &format!("/api/court/{scorer}"), None)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        res.body
    }

    /// The court phone's "Next match" tap.
    async fn advance(&self, scorer: &str) -> Value {
        let res = self
            .anon(Method::POST, &format!("/api/court/{scorer}/advance"), None)
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        res.body
    }

    /// The coordinator types `a-b` for a match: a result, or a correction of one.
    async fn typed(&mut self, match_id: &str, a: u8, b: u8) {
        let client_id = self.client_id();
        let res = self
            .post(
                &format!("/api/events/{}/matches/{match_id}/result", self.id),
                Some(json!({
                    "client_id": client_id,
                    "score": { "sets": [{ "a": a, "b": b, "tiebreak": null }] },
                })),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }

    /// The coordinator flips the winner of a finished match.
    async fn flip(&mut self, match_id: &str) {
        let d = self.detail().await;
        match match_in(&d, match_id)["winner"].as_str().unwrap() {
            "a" => self.typed(match_id, 0, 6).await,
            _ => self.typed(match_id, 6, 0).await,
        }
    }

    async fn point(&mut self, scorer: &str, team: &str) {
        let client_id = self.client_id();
        let res = self
            .anon(
                Method::POST,
                &format!("/api/score/{scorer}/commands"),
                Some(json!({
                    "client_id": client_id, "batch": false, "base_revision": 0,
                    "commands": [{ "kind": "point", "team": team }],
                })),
            )
            .await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    }

    /// The scorer token of the match the court phone of `match_id`'s court shows as current.
    async fn court_scorer(&self, d: &Value, match_id: &str) -> (String, String) {
        let court_id = match_in(d, match_id)["court_id"]
            .as_str()
            .unwrap_or_else(|| panic!("match {match_id} is on no court"));
        let court = d["courts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == json!(court_id))
            .unwrap();
        let court_token = court["scorer_token"].as_str().unwrap().to_owned();
        let state = self.court_state(&court_token).await;
        assert_eq!(
            state["current"]["id"],
            json!(match_id),
            "the court phone shows the match: {state}"
        );
        let match_token = state["current"]["scorer_token"]
            .as_str()
            .unwrap()
            .to_owned();
        (court_token, match_token)
    }

    /// One point to side A of the match that is current on its court: it has started.
    async fn start_match(&mut self, match_id: &str) {
        let d = self.detail().await;
        let (_, token) = self.court_scorer(&d, match_id).await;
        self.point(&token, "a").await;
    }

    /// Score the current match `match_id` on its court phone until a side wins (`a_wins`),
    /// then tap "Next match" when one is queued.
    async fn play(&mut self, match_id: &str, mode: Mode, a_wins: bool) {
        let d = self.detail().await;
        let (court_token, token) = self.court_scorer(&d, match_id).await;
        assert_ne!(
            match_in(&d, match_id)["state"],
            json!("finished"),
            "{match_id} is already finished"
        );
        match mode {
            Mode::Points => {
                let team = if a_wins { "a" } else { "b" };
                let mut finished = false;
                for _ in 0..80 {
                    let view = self
                        .anon(Method::GET, &format!("/api/score/{token}"), None)
                        .await;
                    if view.body["score"]["status"] != json!("in_progress") {
                        finished = true;
                        break;
                    }
                    self.point(&token, team).await;
                }
                assert!(finished, "the match never ended");
            }
            Mode::CourtFinal => {
                let view = self
                    .anon(Method::GET, &format!("/api/score/{token}"), None)
                    .await;
                let (a, b) = if a_wins { (6, 2) } else { (2, 6) };
                let client_id = self.client_id();
                let res = self
                    .anon(
                        Method::POST,
                        &format!("/api/score/{token}/final"),
                        Some(json!({
                            "client_id": client_id,
                            "base_revision": view.body["revision"],
                            "score": { "sets": [{ "a": a, "b": b, "tiebreak": null }] },
                        })),
                    )
                    .await;
                assert_eq!(res.status, StatusCode::OK, "{}", res.body);
            }
            Mode::Coordinator => {
                let (a, b) = if a_wins { (6, 0) } else { (0, 6) };
                self.typed(match_id, a, b).await;
            }
        }
        let d = self.detail().await;
        let m = match_in(&d, match_id);
        assert_eq!(m["state"], json!("finished"), "{m}");
        assert_eq!(m["winner"], json!(if a_wins { "a" } else { "b" }));
        self.advance_if_next(&court_token).await;
    }

    /// Tap "Next match" on a court whose finished match has one queued.
    async fn advance_if_next(&self, court_token: &str) -> bool {
        let state = self.court_state(court_token).await;
        if state["next"].is_null() {
            return false;
        }
        let next = state["next"]["id"].clone();
        let after = self.advance(court_token).await;
        assert_eq!(after["current"]["id"], next, "{after}");
        true
    }

    /// Tap "Next match" on every court that can.
    async fn advance_all(&self) {
        let d = self.detail().await;
        for court in d["courts"].as_array().unwrap() {
            let current_done = match court["current"].as_str() {
                None => true,
                Some(m) => match_in(&d, m)["state"] == json!("finished"),
            };
            if current_done {
                self.advance_if_next(court["scorer_token"].as_str().unwrap())
                    .await;
            }
        }
    }

    /// Play every match to the end. Of the matches on the courts, the one deciding the worst
    /// places goes first, so the final is the last match to be made. The `n`th match is scored
    /// in the way `MODES[n % 3]` says; side A wins every other match.
    async fn play_out(&mut self) {
        let mut n = 0;
        loop {
            self.advance_all().await;
            let d = self.detail().await;
            let mut live: Vec<(u64, i64, String)> = d["courts"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|c| c["current"].as_str())
                .filter(|m| match_in(&d, m)["state"] != json!("finished"))
                .map(|m| {
                    let node = node_of_match(&d, m);
                    (
                        node["place_from"].as_u64().unwrap(),
                        match_in(&d, m)["ordinal"].as_i64().unwrap(),
                        m.to_owned(),
                    )
                })
                .collect();
            if live.is_empty() {
                return;
            }
            live.sort_by_key(|(place_from, ordinal, _)| (std::cmp::Reverse(*place_from), *ordinal));
            let mode = MODES[n % MODES.len()];
            self.play(&live[0].2, mode, n % 2 == 0).await;
            n += 1;
            assert!(n < 100, "the bracket never ends");
        }
    }
}

fn nodes(d: &Value) -> &Vec<Value> {
    d["kind_data"]["nodes"].as_array().unwrap()
}

fn node<'a>(d: &'a Value, label: &str) -> &'a Value {
    nodes(d)
        .iter()
        .find(|n| n["label"] == json!(label))
        .unwrap_or_else(|| panic!("no node {label}"))
}

/// The bracket node of a draw: `place_from`, `size`, `idx`.
fn node_at(d: &Value, place_from: u64, size: u64, idx: u64) -> &Value {
    nodes(d)
        .iter()
        .find(|n| {
            n["place_from"] == json!(place_from)
                && n["size"] == json!(size)
                && n["idx"] == json!(idx)
        })
        .unwrap_or_else(|| panic!("no node ({place_from}, {size}, {idx})"))
}

fn node_of_match<'a>(d: &'a Value, match_id: &str) -> &'a Value {
    nodes(d)
        .iter()
        .find(|n| n["match_id"] == json!(match_id))
        .unwrap_or_else(|| panic!("match {match_id} is in no node"))
}

fn match_id(d: &Value, label: &str) -> String {
    node(d, label)["match_id"].as_str().unwrap().to_owned()
}

fn match_in<'a>(d: &'a Value, match_id: &str) -> &'a Value {
    d["matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == json!(match_id))
        .unwrap_or_else(|| panic!("no match {match_id}"))
}

fn has_match(d: &Value, match_id: &str) -> bool {
    d["matches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["id"] == json!(match_id))
}

fn held_labels(d: &Value) -> Vec<&str> {
    nodes(d)
        .iter()
        .filter(|n| n["held"] == json!(true))
        .map(|n| n["label"].as_str().unwrap())
        .collect()
}

/// What must hold in every state of a running tournament.
fn check_invariants(d: &Value) {
    // No team is on two unfinished matches.
    let mut busy = HashSet::new();
    for m in d["matches"].as_array().unwrap() {
        if m["state"] == json!("finished") {
            continue;
        }
        for team in [&m["team_a"], &m["team_b"]] {
            assert!(
                busy.insert(team.as_str().unwrap()),
                "team on two matches: {d}"
            );
        }
    }
    // A court has at most one match queued behind its current one, and no match is on two
    // courts.
    let mut queued = HashSet::new();
    for c in d["courts"].as_array().unwrap() {
        let queue = c["queue"].as_array().unwrap();
        assert!(queue.len() <= 1, "court queue too long: {c}");
        for m in queue
            .iter()
            .chain(c["current"].as_str().map(|_| &c["current"]))
        {
            assert!(
                queued.insert(m.as_str().unwrap()),
                "match on two courts: {d}"
            );
        }
    }
}

async fn player(ctx: &Ctx, name: &str) -> String {
    let res = ctx
        .post("/api/players", Some(json!({ "name": name })))
        .await;
    assert!(res.status.is_success(), "{}", res.body);
    res.body["id"].as_str().unwrap().to_owned()
}

/// Team `i`, of two made-up players.
async fn add_team(ctx: &Ctx, i: usize) {
    let a = player(ctx, &format!("Ana{i}")).await;
    let b = player(ctx, &format!("Rui{i}")).await;
    let res = ctx
        .post(
            &format!("/api/events/{}/teams", ctx.id),
            Some(json!({ "players": [a, b] })),
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

async fn add_court(ctx: &Ctx) {
    let res = ctx
        .post(&format!("/api/events/{}/courts", ctx.id), Some(json!({})))
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
}

/// A tournament in setup with no teams and two courts. `full_placement` of `None` leaves the
/// settings out, so the default applies.
async fn new_event(name: &str, full_placement: Option<bool>) -> (Ctx, SqlitePool) {
    let (state, pool, app, _admin, raw) = with_admin().await;
    let mut body = json!({ "name": name, "kind": "tournament", "format": one_set() });
    if let Some(on) = full_placement {
        body["settings"] = json!({ "full_placement": on });
    }
    let res = send(
        &app,
        as_session(&state, &raw, Method::POST, "/api/events", Some(body)),
    )
    .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.body);
    let id = res.body["event"]["id"].as_str().unwrap().to_owned();
    let ctx = Ctx {
        state,
        app,
        raw,
        id,
        calls: 0,
    };
    add_court(&ctx).await;
    add_court(&ctx).await;
    (ctx, pool)
}

/// The places are a gapless tiling of `1..=n`, each team once.
fn assert_tiles(places: &Value, n: usize) {
    let places = places.as_array().unwrap();
    assert_eq!(places.len(), n, "{places:?}");
    let teams: HashSet<&str> = places
        .iter()
        .map(|p| p["team_id"].as_str().unwrap())
        .collect();
    assert_eq!(teams.len(), n, "a team twice: {places:?}");
    let mut ranges: Vec<(u64, u64)> = places
        .iter()
        .map(|p| {
            (
                p["place"].as_u64().unwrap(),
                p["place_to"].as_u64().unwrap(),
            )
        })
        .collect();
    ranges.sort_unstable();
    ranges.dedup();
    let mut next = 1;
    for (from, to) in ranges {
        assert_eq!(from, next, "gap or overlap: {places:?}");
        next = to + 1;
    }
    assert_eq!(next, n as u64 + 1, "{places:?}");
}

/// The team that won the match of a node.
fn winner_team(d: &Value, node: &Value) -> String {
    let m = match_in(d, node["match_id"].as_str().unwrap());
    match m["winner"].as_str().unwrap() {
        "a" => m["team_a"].as_str().unwrap().to_owned(),
        _ => m["team_b"].as_str().unwrap().to_owned(),
    }
}

fn place_of(places: &Value, place: u64) -> &str {
    places
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["place"] == json!(place))
        .unwrap()["team_id"]
        .as_str()
        .unwrap()
}

/// The final is the last bracket match to be made.
fn assert_final_is_last_made(d: &Value) {
    let final_match = node_at(d, 1, 2, 0)["match_id"].as_str().unwrap();
    let final_ordinal = match_in(d, final_match)["ordinal"].as_i64().unwrap();
    for m in d["matches"].as_array().unwrap() {
        if m["id"] != json!(final_match) {
            assert!(
                m["ordinal"].as_i64().unwrap() < final_ordinal,
                "{} was made after the final: {m}",
                m["label"]
            );
        }
    }
}

async fn placing_rows(pool: &SqlitePool, event_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM placings WHERE event_id = ?")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn all_strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
    match v {
        Value::String(s) => out.push(s),
        Value::Array(items) => items.iter().for_each(|i| all_strings(i, out)),
        Value::Object(map) => {
            for (k, v) in map {
                out.push(k);
                all_strings(v, out);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn eight_team_two_court_tournament_runs_end_to_end() {
    let (mut ctx, pool) = new_event("Sunday cup", Some(true)).await;

    // Six teams can't start; eight can.
    for i in 0..6 {
        add_team(&ctx, i).await;
    }
    let start = format!("/api/events/{}/start", ctx.id);
    let res = ctx.post(&start, None).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "team_count" }));
    for i in 6..8 {
        add_team(&ctx, i).await;
    }
    let res = ctx.post(&start, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);

    // The bracket: 12 matches, four quarter-finals made, two courts busy.
    let d = ctx.detail().await;
    assert_eq!(nodes(&d).len(), 12);
    assert_eq!(d["kind_data"]["can_redraw"], json!(true));
    let old_quarters: Vec<String> = (1..=4)
        .map(|i| match_id(&d, &format!("Quarter-final {i}")))
        .collect();
    assert_eq!(d["matches"].as_array().unwrap().len(), 4);

    // Redraw once: the quarter-finals are made again.
    let res = ctx
        .post(&format!("/api/events/{}/bracket/redraw", ctx.id), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let d = ctx.detail().await;
    let quarters: Vec<String> = (1..=4)
        .map(|i| match_id(&d, &format!("Quarter-final {i}")))
        .collect();
    for old in &old_quarters {
        assert!(
            !has_match(&d, old),
            "an old quarter-final survived the redraw"
        );
    }
    assert_eq!(d["matches"].as_array().unwrap().len(), 4);
    let (qf1, qf2, qf3, qf4) = (&quarters[0], &quarters[1], &quarters[2], &quarters[3]);

    // The first two are on the courts, the others wait for a free one.
    assert_ne!(match_in(&d, qf1)["court_id"], Value::Null);
    assert_ne!(match_in(&d, qf2)["court_id"], Value::Null);
    assert_eq!(match_in(&d, qf3)["court_id"], Value::Null);
    assert_eq!(match_in(&d, qf4)["court_id"], Value::Null);

    // Quarter-finals 1 and 2, by points and by a typed final on the court phone; side A wins.
    ctx.play(qf1, Mode::Points, true).await;
    ctx.play(qf2, Mode::CourtFinal, true).await;
    let d = ctx.detail().await;
    assert_eq!(d["kind_data"]["can_redraw"], json!(false));
    let sf1 = match_id(&d, "Semi-final 1");
    let ps1 = match_id(&d, "Placement semi 1 (5th–8th)");
    assert_eq!(
        node(&d, "Semi-final 1")["teams"][0],
        node(&d, "Quarter-final 1")["teams"][0],
        "side A of quarter-final 1 won"
    );

    // A correction before the next match started: it is made again, with no hold.
    ctx.flip(qf1).await;
    let d = ctx.detail().await;
    assert!(held_labels(&d).is_empty(), "{:?}", held_labels(&d));
    let (new_sf1, new_ps1) = (
        match_id(&d, "Semi-final 1"),
        match_id(&d, "Placement semi 1 (5th–8th)"),
    );
    assert_ne!(new_sf1, sf1);
    assert_ne!(new_ps1, ps1);
    assert!(!has_match(&d, &sf1) && !has_match(&d, &ps1));
    let qf1_teams = &node(&d, "Quarter-final 1")["teams"];
    assert_eq!(node(&d, "Semi-final 1")["teams"][0], qf1_teams[1]);
    assert_eq!(
        node(&d, "Placement semi 1 (5th–8th)")["teams"][0],
        qf1_teams[0]
    );
    let sf1 = new_sf1;

    // Quarter-finals 3 and 4; the semi-finals of the other half are made.
    ctx.play(qf3, Mode::Coordinator, true).await;
    ctx.play(qf4, Mode::Points, false).await;
    let d = ctx.detail().await;
    let sf2 = match_id(&d, "Semi-final 2");
    let ps2 = match_id(&d, "Placement semi 2 (5th–8th)");

    // Semi-final 1 and placement semi 1 are on the courts. A point on semi-final 1, then a
    // correction to quarter-final 2 that changes its winner: held, the coordinator keeps it.
    ctx.start_match(&sf1).await;
    ctx.flip(qf2).await;
    let d = ctx.detail().await;
    assert_eq!(held_labels(&d), ["Quarter-final 2"]);
    assert_eq!(
        node(&d, "Quarter-final 2")["replay_discards"],
        json!(["Semi-final 1", "Placement semi 1 (5th–8th)"])
    );
    assert_eq!(
        match_id(&d, "Semi-final 1"),
        sf1,
        "nothing downstream changed"
    );
    let finish = format!("/api/events/{}/finish", ctx.id);
    let res = ctx.post(&finish, Some(json!({}))).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.body, json!({ "error": "bracket_needs_decision" }));
    let res = ctx
        .post(
            &format!("/api/events/{}/bracket/decisions/{qf2}", ctx.id),
            Some(json!({ "choice": "keep" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let d = ctx.detail().await;
    assert!(held_labels(&d).is_empty());
    assert_eq!(match_id(&d, "Semi-final 1"), sf1, "kept as played");
    assert_eq!(match_in(&d, &sf1)["state"], json!("live"));

    // Play semi-final 1 on, then placement semi 1, which brings the other half onto the courts.
    ctx.play(&sf1, Mode::Points, true).await;
    let d = ctx.detail().await;
    let ps1 = match_id(&d, "Placement semi 1 (5th–8th)");
    ctx.play(&ps1, Mode::CourtFinal, false).await;
    let d = ctx.detail().await;
    assert_eq!(match_id(&d, "Semi-final 2"), sf2);
    assert_eq!(match_id(&d, "Placement semi 2 (5th–8th)"), ps2);

    // Placement semi 2 has started; correct quarter-final 3 so its winner flips: held, replayed.
    let old_ps2_court = match_in(&d, &ps2)["court_id"].as_str().unwrap().to_owned();
    ctx.start_match(&ps2).await;
    let d = ctx.detail().await;
    let old_sf2_token = match_in(&d, &sf2)["scorer_token"]
        .as_str()
        .unwrap()
        .to_owned();
    ctx.flip(qf3).await;
    let d = ctx.detail().await;
    assert_eq!(held_labels(&d), ["Quarter-final 3"]);
    assert_eq!(
        node(&d, "Quarter-final 3")["replay_discards"],
        json!(["Semi-final 2", "Placement semi 2 (5th–8th)"])
    );
    let res = ctx
        .post(
            &format!("/api/events/{}/bracket/decisions/{qf3}", ctx.id),
            Some(json!({ "choice": "replay" })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let d = ctx.detail().await;
    assert!(held_labels(&d).is_empty());
    // The discarded matches are gone and made again with the right teams.
    assert!(!has_match(&d, &sf2) && !has_match(&d, &ps2));
    let (new_sf2, new_ps2) = (
        match_id(&d, "Semi-final 2"),
        match_id(&d, "Placement semi 2 (5th–8th)"),
    );
    assert_ne!(new_sf2, sf2);
    assert_ne!(new_ps2, ps2);
    let qf3_teams = &node(&d, "Quarter-final 3")["teams"];
    assert_eq!(
        node(&d, "Semi-final 2")["teams"][0],
        qf3_teams[1],
        "the corrected winner"
    );
    assert_eq!(
        node(&d, "Placement semi 2 (5th–8th)")["teams"][0],
        qf3_teams[0]
    );
    // The old scoring link is dead, and the court phone that showed placement semi 2 has moved
    // on to a match that exists.
    let gone = ctx
        .anon(Method::GET, &format!("/api/score/{old_sf2_token}"), None)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    let court = d["courts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == json!(old_ps2_court))
        .unwrap();
    let phone = ctx
        .court_state(court["scorer_token"].as_str().unwrap())
        .await;
    assert_ne!(phone["current"]["id"], json!(ps2), "{phone}");
    if let Some(now) = phone["current"]["id"].as_str() {
        assert!(has_match(&d, now));
    }

    // Everything else, mixing the three ways to score.
    ctx.play_out().await;
    let d = ctx.detail().await;
    assert_eq!(d["matches"].as_array().unwrap().len(), 12);
    assert!(
        d["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["state"] == json!("finished"))
    );
    assert_final_is_last_made(&d);
    assert!(held_labels(&d).is_empty());
    assert_tiles(&d["kind_data"]["places"], 8);

    // Finish with no places: the bracket decides, 16 placings, one per player.
    let res = ctx.post(&finish, Some(json!({}))).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_eq!(res.body["event"]["status"], json!("finished"));
    let places = res.body["places"].clone();
    assert_tiles(&places, 8);
    for p in places.as_array().unwrap() {
        assert_eq!(p["place"], p["place_to"], "full placement ties nobody: {p}");
    }
    assert_eq!(placing_rows(&pool, &ctx.id).await, 16);
    let d = ctx.detail().await;
    let final_node = node_at(&d, 1, 2, 0).clone();
    let final_winner = winner_team(&d, &final_node);
    assert_eq!(place_of(&places, 1), final_winner);

    // The spectator link: the bracket and the final places, and no scorer token anywhere.
    let spectator = d["spectator_token"].as_str().unwrap();
    let view = ctx
        .anon(Method::GET, &format!("/api/event-view/{spectator}"), None)
        .await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.body);
    assert_eq!(
        view.body["kind_data"]["nodes"].as_array().unwrap().len(),
        12
    );
    assert_tiles(&view.body["places"], 8);
    assert_eq!(view.body["places"], places);
    let mut secrets: Vec<&str> = d["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["scorer_token"].as_str().unwrap())
        .collect();
    secrets.extend(
        d["courts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["scorer_token"].as_str().unwrap()),
    );
    let mut found = Vec::new();
    all_strings(&view.body, &mut found);
    for secret in &secrets {
        assert!(
            !found.contains(secret),
            "a scorer token leaked: {}",
            view.body
        );
    }
    assert!(
        !found.iter().any(|s| s.contains("scorer")),
        "a scorer field leaked: {}",
        view.body
    );

    // Correct the final after Finish: places 1 and 2 swap.
    let final_match = final_node["match_id"].as_str().unwrap().to_owned();
    let runner_up = place_of(&places, 2).to_owned();
    ctx.flip(&final_match).await;
    let d = ctx.detail().await;
    assert_eq!(d["event"]["status"], json!("finished"));
    assert_eq!(place_of(&d["places"], 1), runner_up);
    assert_eq!(place_of(&d["places"], 2), final_winner);
    assert_tiles(&d["places"], 8);
    assert_eq!(placing_rows(&pool, &ctx.id).await, 16);
    let view = ctx
        .anon(Method::GET, &format!("/api/event-view/{spectator}"), None)
        .await;
    assert_eq!(place_of(&view.body["places"], 1), runner_up);
    assert_eq!(place_of(&view.body["places"], 2), final_winner);
}

#[tokio::test]
async fn four_team_off_mode_runs_end_to_end() {
    // Full placement is off by default.
    let (mut ctx, pool) = new_event("Mini cup", None).await;
    for i in 0..4 {
        add_team(&ctx, i).await;
    }
    let res = ctx
        .post(&format!("/api/events/{}/start", ctx.id), None)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    let d = ctx.detail().await;
    assert_eq!(d["kind_data"]["full_placement"], json!(false));
    // Two semi-finals, the final and the 3rd place match.
    assert_eq!(nodes(&d).len(), 4);
    assert_eq!(d["matches"].as_array().unwrap().len(), 2);

    ctx.play_out().await;
    let d = ctx.detail().await;
    assert_eq!(d["matches"].as_array().unwrap().len(), 4);
    assert!(
        d["matches"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["state"] == json!("finished"))
    );
    assert_final_is_last_made(&d);

    let res = ctx
        .post(&format!("/api/events/{}/finish", ctx.id), Some(json!({})))
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.body);
    assert_tiles(&res.body["places"], 4);
    for p in res.body["places"].as_array().unwrap() {
        assert_eq!(p["place"], p["place_to"], "{p}");
    }
    assert_eq!(placing_rows(&pool, &ctx.id).await, 8);
    let d = ctx.detail().await;
    let final_winner = winner_team(&d, node_at(&d, 1, 2, 0));
    let third_winner = winner_team(&d, node_at(&d, 3, 2, 0));
    assert_eq!(place_of(&d["places"], 1), final_winner);
    assert_eq!(place_of(&d["places"], 3), third_winner);
}
