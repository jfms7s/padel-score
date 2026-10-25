#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use padel_score_server::accounts_store::{
    DEVICE_CODE_TTL_SECS, InviteStatus, MAX_ACTIVE_INVITES, NewUser, PollOutcome, SESSION_TTL_SECS,
    normalize_user_code,
};
use padel_score_server::db;
use padel_score_server::store::{NewMatch, Store, StoreError, Teams};
use padel_scoring::{DecidingSet, DeuceRule, Format, Player, SetRules, SetsFormat, Start, Team};
use sqlx::{AssertSqlSafe, Row, SqlitePool};

const DAY: i64 = 24 * 3600;
const NOW: i64 = 1_000 * DAY;

async fn setup() -> (Store, SqlitePool) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    (Store::new(pool.clone()), pool)
}

fn new_user(email: &str) -> NewUser {
    NewUser {
        email: email.into(),
        display_name: format!("User {email}"),
        password_hash: "$argon2id$fake".into(),
    }
}

fn new_match(owner: Option<&str>) -> NewMatch {
    NewMatch {
        format: Format::Sets(SetRules {
            deuce: DeuceRule::Advantage,
            sets: SetsFormat::BestOf3,
            deciding: DecidingSet::FullSet,
        }),
        start: Start {
            first_server: Player {
                team: Team::A,
                index: 0,
            },
            other_first_server: 0,
        },
        teams: Teams {
            a: ["Ana".into(), "Bea".into()],
            b: ["Cris".into(), "Dani".into()],
        },
        owner_user_id: owner.map(str::to_owned),
    }
}

/// An admin plus a regular user.
async fn admin_and_user(s: &Store) -> (String, String) {
    let admin = s
        .create_first_admin(new_user("admin@x.test"), NOW)
        .await
        .unwrap();
    let user = s.create_user(new_user("user@x.test"), NOW).await.unwrap();
    (admin.id, user.id)
}

/// A device token issued to `user_id` through the whole device flow.
async fn device_token(s: &Store, user_id: &str, now: i64) -> String {
    let started = s.start_device("Watch", now).await.unwrap();
    s.approve_device(&started.user_code, user_id, now)
        .await
        .unwrap();
    match s.poll_device(&started.device_code, now).await.unwrap() {
        PollOutcome::Issued { raw_token } => raw_token,
        other => panic!("expected a token, got {other:?}"),
    }
}

fn is_conflict(r: &Result<impl std::fmt::Debug, StoreError>, code: &str) -> bool {
    matches!(r, Err(StoreError::Conflict(c)) if *c == code)
}

#[tokio::test]
async fn email_is_unique_case_insensitively() {
    let (s, _) = setup().await;
    let u = s
        .create_user(new_user("  Ana@Example.TEST "), NOW)
        .await
        .unwrap();
    assert_eq!(u.email, "ana@example.test");
    let dup = s.create_user(new_user("ana@example.test"), NOW).await;
    assert!(is_conflict(&dup, "email_taken"), "{dup:?}");
    let dup = s.create_user(new_user("ANA@EXAMPLE.TEST"), NOW).await;
    assert!(is_conflict(&dup, "email_taken"), "{dup:?}");
    let found = s.user_by_email(" ANA@example.test").await.unwrap().unwrap();
    assert_eq!(found.user.id, u.id);
    assert_eq!(found.password_hash, "$argon2id$fake");
    assert_eq!(s.user_by_id(&u.id).await.unwrap().unwrap(), u);
    assert!(s.user_by_email("nobody@x.test").await.unwrap().is_none());
    assert_eq!(s.user_count().await.unwrap(), 1);
}

#[tokio::test]
async fn first_admin_only_once() {
    let (s, _) = setup().await;
    assert_eq!(s.user_count().await.unwrap(), 0);
    let admin = s
        .create_first_admin(new_user("admin@x.test"), NOW)
        .await
        .unwrap();
    assert!(admin.is_admin);
    assert!(admin.invited_by.is_none());
    let again = s.create_first_admin(new_user("other@x.test"), NOW).await;
    assert!(is_conflict(&again, "setup_done"), "{again:?}");
    let plain = s.create_user(new_user("plain@x.test"), NOW).await.unwrap();
    assert!(!plain.is_admin);
    assert_eq!(s.user_count().await.unwrap(), 2);
}

#[tokio::test]
async fn first_admin_refused_when_any_user_exists() {
    let (s, _) = setup().await;
    s.create_user(new_user("plain@x.test"), NOW).await.unwrap();
    let r = s.create_first_admin(new_user("admin@x.test"), NOW).await;
    assert!(is_conflict(&r, "setup_done"), "{r:?}");
}

#[tokio::test]
async fn only_hashes_are_stored() {
    let (s, pool) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    let mut secrets = vec![s.create_session(&user, NOW).await.unwrap()];
    let invite = s.create_invite(&user, NOW).await.unwrap();
    secrets.push(invite.code.clone());
    secrets.push(s.create_reset(&user, &admin, NOW).await.unwrap());
    let started = s.start_device("Watch", NOW).await.unwrap();
    secrets.push(started.device_code.clone());
    s.approve_device(&started.user_code, &user, NOW)
        .await
        .unwrap();
    let PollOutcome::Issued { raw_token } = s.poll_device(&started.device_code, NOW).await.unwrap()
    else {
        panic!("expected a token");
    };
    assert!(raw_token.starts_with("pdl_"));
    secrets.push(raw_token);
    // A device code still pending, too.
    secrets.push(s.start_device("Other", NOW).await.unwrap().device_code);

    let tables: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
            .fetch_all(&pool)
            .await
            .unwrap();
    let mut scanned = 0;
    for table in &tables {
        let columns = sqlx::query(AssertSqlSafe(format!("PRAGMA table_info({table})")))
            .fetch_all(&pool)
            .await
            .unwrap();
        for col in columns {
            let ty: String = col.get("type");
            if !ty.eq_ignore_ascii_case("TEXT") {
                continue;
            }
            let name: String = col.get("name");
            let values: Vec<Option<String>> =
                sqlx::query_scalar(AssertSqlSafe(format!("SELECT \"{name}\" FROM \"{table}\"")))
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            for value in values.into_iter().flatten() {
                scanned += 1;
                for secret in &secrets {
                    assert!(
                        !value.contains(secret.as_str()),
                        "{table}.{name} holds a raw secret"
                    );
                }
            }
        }
    }
    assert!(scanned > 20, "scanned only {scanned} values");
}

#[tokio::test]
async fn session_expires_after_30_days() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let raw = s.create_session(&user, NOW).await.unwrap();
    let got = s.session_user(&raw, NOW).await.unwrap().unwrap();
    assert_eq!(got.id, user);
    let last = NOW + SESSION_TTL_SECS - 1;
    assert!(s.session_user(&raw, last).await.unwrap().is_some());
    let end = NOW + SESSION_TTL_SECS;
    assert!(s.session_user(&raw, end).await.unwrap().is_none());
    assert!(s.session_user("garbage", NOW).await.unwrap().is_none());
    assert_eq!(s.purge_expired_sessions(end - 1).await.unwrap(), 0);
    assert_eq!(s.purge_expired_sessions(end).await.unwrap(), 1);
}

#[tokio::test]
async fn sessions_can_be_deleted_one_or_all_but_one() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let a = s.create_session(&user, NOW).await.unwrap();
    let b = s.create_session(&user, NOW).await.unwrap();
    let c = s.create_session(&user, NOW).await.unwrap();
    s.delete_session(&a).await.unwrap();
    assert!(s.session_user(&a, NOW).await.unwrap().is_none());
    s.delete_sessions_except(&user, &b).await.unwrap();
    assert!(s.session_user(&b, NOW).await.unwrap().is_some());
    assert!(s.session_user(&c, NOW).await.unwrap().is_none());
}

#[tokio::test]
async fn disabled_user_session_is_none() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let raw = s.create_session(&user, NOW).await.unwrap();
    s.set_disabled(&user, Some(NOW)).await.unwrap();
    assert!(s.session_user(&raw, NOW).await.unwrap().is_none());
    let u = s.user_by_id(&user).await.unwrap().unwrap();
    assert_eq!(u.disabled_at, Some(NOW));
    s.set_disabled(&user, None).await.unwrap();
    assert!(
        s.user_by_id(&user)
            .await
            .unwrap()
            .unwrap()
            .disabled_at
            .is_none()
    );
    assert!(matches!(
        s.set_disabled("missing", Some(NOW)).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn disabling_revokes_sessions_and_device_tokens() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let session = s.create_session(&user, NOW).await.unwrap();
    let token = device_token(&s, &user, NOW).await;
    assert!(s.token_user(&token, NOW).await.unwrap().is_some());
    s.set_disabled(&user, Some(NOW)).await.unwrap();
    s.set_disabled(&user, None).await.unwrap();
    // Re-enabling does not bring either back.
    assert!(s.session_user(&session, NOW).await.unwrap().is_none());
    assert!(s.token_user(&token, NOW).await.unwrap().is_none());
    assert!(s.list_devices(&user).await.unwrap().is_empty());
}

#[tokio::test]
async fn last_admin_cannot_be_disabled() {
    let (s, pool) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    let r = s.set_disabled(&admin, Some(NOW)).await;
    assert!(is_conflict(&r, "last_admin"), "{r:?}");
    assert!(
        s.user_by_id(&admin)
            .await
            .unwrap()
            .unwrap()
            .disabled_at
            .is_none()
    );
    // With a second enabled admin, the first can go; then the second is the last.
    sqlx::query("UPDATE users SET is_admin = 1 WHERE id = ?")
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
    s.set_disabled(&admin, Some(NOW)).await.unwrap();
    let r = s.set_disabled(&user, Some(NOW)).await;
    assert!(is_conflict(&r, "last_admin"), "{r:?}");
}

#[tokio::test]
async fn set_password_revokes_sessions_and_device_tokens() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let session = s.create_session(&user, NOW).await.unwrap();
    let token = device_token(&s, &user, NOW).await;
    s.set_password(&user, "$argon2id$new", NOW).await.unwrap();
    assert!(s.session_user(&session, NOW).await.unwrap().is_none());
    assert!(s.token_user(&token, NOW).await.unwrap().is_none());
    let creds = s.user_by_email("user@x.test").await.unwrap().unwrap();
    assert_eq!(creds.password_hash, "$argon2id$new");
    assert!(matches!(
        s.set_password("missing", "h", NOW).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn invite_single_use() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let invite = s.create_invite(&user, NOW).await.unwrap();
    assert_eq!(invite.expires_at, NOW + 7 * DAY);
    let info = s.invite_by_code(&invite.code, NOW).await.unwrap().unwrap();
    assert_eq!(info.status, InviteStatus::Valid);
    assert_eq!(info.created_by_name, "User user@x.test");
    let joined = s
        .accept_invite(&invite.code, new_user(" New@X.test"), NOW)
        .await
        .unwrap();
    assert_eq!(joined.invited_by.as_deref(), Some(user.as_str()));
    assert_eq!(joined.email, "new@x.test");
    assert!(!joined.is_admin);
    let info = s.invite_by_code(&invite.code, NOW).await.unwrap().unwrap();
    assert_eq!(info.status, InviteStatus::Used);
    let again = s
        .accept_invite(&invite.code, new_user("second@x.test"), NOW)
        .await;
    assert!(is_conflict(&again, "invite_used"), "{again:?}");
    assert!(s.user_by_email("second@x.test").await.unwrap().is_none());
    let listed = s.list_invites(&user).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, invite.id);
    assert_eq!(listed[0].used_by_name.as_deref(), Some("User  New@X.test"));
    assert_eq!(listed[0].status(NOW), InviteStatus::Used);
    assert!(s.invite_by_code("garbage", NOW).await.unwrap().is_none());
    assert!(matches!(
        s.accept_invite("garbage", new_user("g@x.test"), NOW).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn invite_with_taken_email_stays_unused() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let invite = s.create_invite(&user, NOW).await.unwrap();
    let r = s
        .accept_invite(&invite.code, new_user("ADMIN@x.test"), NOW)
        .await;
    assert!(is_conflict(&r, "email_taken"), "{r:?}");
    let info = s.invite_by_code(&invite.code, NOW).await.unwrap().unwrap();
    assert_eq!(info.status, InviteStatus::Valid);
}

#[tokio::test]
async fn invite_expired_revoked_and_disabled_creator_rejected() {
    let (s, _) = setup().await;
    let (admin, user) = admin_and_user(&s).await;

    let expiring = s.create_invite(&user, NOW).await.unwrap();
    let later = NOW + 7 * DAY;
    let info = s
        .invite_by_code(&expiring.code, later)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(info.status, InviteStatus::Expired);
    let r = s
        .accept_invite(&expiring.code, new_user("a@x.test"), later)
        .await;
    assert!(is_conflict(&r, "invite_expired"), "{r:?}");

    let revoked = s.create_invite(&user, NOW).await.unwrap();
    assert!(matches!(
        s.revoke_invite(&admin, &revoked.id, NOW).await,
        Err(StoreError::NotFound)
    ));
    s.revoke_invite(&user, &revoked.id, NOW).await.unwrap();
    let info = s.invite_by_code(&revoked.code, NOW).await.unwrap().unwrap();
    assert_eq!(info.status, InviteStatus::Revoked);
    let r = s
        .accept_invite(&revoked.code, new_user("b@x.test"), NOW)
        .await;
    assert!(is_conflict(&r, "invite_revoked"), "{r:?}");

    let orphan = s.create_invite(&user, NOW).await.unwrap();
    s.set_disabled(&user, Some(NOW)).await.unwrap();
    let info = s.invite_by_code(&orphan.code, NOW).await.unwrap().unwrap();
    assert_eq!(info.status, InviteStatus::Revoked);
    let r = s
        .accept_invite(&orphan.code, new_user("c@x.test"), NOW)
        .await;
    assert!(is_conflict(&r, "invite_revoked"), "{r:?}");
    assert_eq!(s.user_count().await.unwrap(), 2);
}

#[tokio::test]
async fn twenty_active_invites_limit() {
    let (s, _) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    let mut ids = Vec::new();
    for _ in 0..MAX_ACTIVE_INVITES {
        ids.push(s.create_invite(&user, NOW).await.unwrap().id);
    }
    let r = s.create_invite(&user, NOW).await;
    assert!(is_conflict(&r, "too_many_invites"), "{r:?}");
    // Another user has their own allowance.
    s.create_invite(&admin, NOW).await.unwrap();
    // A revoked invite frees a slot, and so does an expired one.
    s.revoke_invite(&user, &ids[0], NOW).await.unwrap();
    s.create_invite(&user, NOW).await.unwrap();
    assert!(is_conflict(
        &s.create_invite(&user, NOW).await,
        "too_many_invites"
    ));
    s.create_invite(&user, NOW + 7 * DAY).await.unwrap();
}

#[tokio::test]
async fn reset_single_use_revokes_everything() {
    let (s, _) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    let session = s.create_session(&user, NOW).await.unwrap();
    let token = device_token(&s, &user, NOW).await;
    let code = s.create_reset(&user, &admin, NOW).await.unwrap();
    let got = s
        .consume_reset(&code, "$argon2id$reset", NOW)
        .await
        .unwrap();
    assert_eq!(got, user);
    assert!(s.session_user(&session, NOW).await.unwrap().is_none());
    assert!(s.token_user(&token, NOW).await.unwrap().is_none());
    let creds = s.user_by_email("user@x.test").await.unwrap().unwrap();
    assert_eq!(creds.password_hash, "$argon2id$reset");
    assert!(matches!(
        s.consume_reset(&code, "$argon2id$again", NOW).await,
        Err(StoreError::NotFound)
    ));
    // 24 hours, then it is gone.
    let code = s.create_reset(&user, &admin, NOW).await.unwrap();
    assert!(matches!(
        s.consume_reset(&code, "h", NOW + DAY).await,
        Err(StoreError::NotFound)
    ));
    let code = s.create_reset(&user, &admin, NOW).await.unwrap();
    s.consume_reset(&code, "h", NOW + DAY - 1).await.unwrap();
    assert!(matches!(
        s.create_reset("missing", &admin, NOW).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn device_poll_lifecycle() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let started = s.start_device("Ana's watch", NOW).await.unwrap();
    assert_eq!(started.expires_at, NOW + DEVICE_CODE_TTL_SECS);
    let poll = |t: i64| s.poll_device(&started.device_code, t);
    assert_eq!(poll(NOW).await.unwrap(), PollOutcome::Pending);
    assert_eq!(poll(NOW + 4).await.unwrap(), PollOutcome::SlowDown);
    // The too-fast poll still counts, so the next must wait 5 s from it.
    assert_eq!(poll(NOW + 8).await.unwrap(), PollOutcome::SlowDown);
    assert_eq!(poll(NOW + 13).await.unwrap(), PollOutcome::Pending);
    let shown = s
        .device_by_user_code(&started.user_code)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(shown.device_name, "Ana's watch");
    assert_eq!(shown.created_at, NOW);
    s.approve_device(&started.user_code, &user, NOW + 14)
        .await
        .unwrap();
    let r = s.approve_device(&started.user_code, &user, NOW + 14).await;
    assert!(is_conflict(&r, "device_code_used"), "{r:?}");
    let PollOutcome::Issued { raw_token } = poll(NOW + 20).await.unwrap() else {
        panic!("expected a token");
    };
    assert_eq!(poll(NOW + 30).await.unwrap(), PollOutcome::Consumed);
    let got = s.token_user(&raw_token, NOW + 30).await.unwrap().unwrap();
    assert_eq!(got.id, user);
    let devices = s.list_devices(&user).await.unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "Ana's watch");
    assert_eq!(devices[0].prefix, raw_token[..8]);
    s.revoke_device(&user, &devices[0].id, NOW + 40)
        .await
        .unwrap();
    assert!(s.token_user(&raw_token, NOW + 40).await.unwrap().is_none());
    assert!(matches!(
        s.revoke_device(&user, &devices[0].id, NOW + 40).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.poll_device("garbage", NOW).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn device_code_expires_after_10_minutes() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let started = s.start_device("Watch", NOW).await.unwrap();
    let end = NOW + DEVICE_CODE_TTL_SECS;
    assert!(matches!(
        s.approve_device(&started.user_code, &user, end).await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        s.poll_device(&started.device_code, end).await.unwrap(),
        PollOutcome::Expired
    );
    // Approved a second before expiry but not polled in time: the code is dead at 10 minutes.
    let late = s.start_device("Watch", NOW).await.unwrap();
    s.approve_device(&late.user_code, &user, end - 1)
        .await
        .unwrap();
    assert_eq!(
        s.poll_device(&late.device_code, end).await.unwrap(),
        PollOutcome::Expired
    );
}

#[tokio::test]
async fn denied_device_is_denied() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let started = s.start_device("Watch", NOW).await.unwrap();
    s.deny_device(&started.user_code, NOW).await.unwrap();
    assert_eq!(
        s.poll_device(&started.device_code, NOW + 10).await.unwrap(),
        PollOutcome::Denied
    );
    let r = s.approve_device(&started.user_code, &user, NOW).await;
    assert!(is_conflict(&r, "device_code_used"), "{r:?}");
    assert!(matches!(
        s.deny_device("ZZZZ-ZZZZ", NOW).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn user_code_lookup_is_forgiving() {
    let (s, _) = setup().await;
    let started = s.start_device("Watch", NOW).await.unwrap();
    let code = &started.user_code;
    assert_eq!(code.len(), 9);
    assert_eq!(code.as_bytes()[4], b'-');
    assert!(
        code.chars()
            .filter(|&c| c != '-')
            .all(|c| "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(c))
    );
    let bare: String = code.chars().filter(|&c| c != '-').collect();
    let sloppy = format!(" {} {} ", bare[..4].to_lowercase(), &bare[4..]);
    for input in [code.clone(), bare.clone(), sloppy, code.to_lowercase()] {
        let found = s.device_by_user_code(&input).await.unwrap();
        assert_eq!(found.map(|d| d.user_code), Some(code.clone()), "{input:?}");
    }
    assert_eq!(
        normalize_user_code("abcd-2345").as_deref(),
        Some("ABCD-2345")
    );
    assert_eq!(
        normalize_user_code("ab cd 23 45").as_deref(),
        Some("ABCD-2345")
    );
    assert!(normalize_user_code("ABCD-234").is_none());
    assert!(normalize_user_code("ABCD-23450").is_none());
    assert!(normalize_user_code("ABCD-I0O1").is_none());
    assert!(s.device_by_user_code("nonsense").await.unwrap().is_none());
}

#[tokio::test]
async fn device_token_last_used_written_at_most_once_a_minute() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let token = device_token(&s, &user, NOW).await;
    let last_used = || async { s.list_devices(&user).await.unwrap()[0].last_used_at };
    assert_eq!(last_used().await, None);
    s.token_user(&token, NOW + 100).await.unwrap().unwrap();
    assert_eq!(last_used().await, Some(NOW + 100));
    s.token_user(&token, NOW + 159).await.unwrap().unwrap();
    assert_eq!(last_used().await, Some(NOW + 100));
    s.token_user(&token, NOW + 160).await.unwrap().unwrap();
    assert_eq!(last_used().await, Some(NOW + 160));
    assert!(s.token_user("pdl_garbage", NOW).await.unwrap().is_none());
}

#[tokio::test]
async fn owner_user_id_round_trips_on_matches() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let m = s.create_match(new_match(Some(&user)), NOW).await.unwrap();
    assert_eq!(m.owner_user_id.as_deref(), Some(user.as_str()));
    let by_scorer = s.by_scorer_token(&m.scorer_token).await.unwrap().unwrap();
    assert_eq!(by_scorer, m);
    let by_spectator = s
        .by_spectator_token(&m.spectator_token)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_spectator, m);
    let (snap, _) = s.snapshot(&m.id).await.unwrap().unwrap();
    assert_eq!(snap, m);
    assert_eq!(
        s.match_owned_by(&m.id, &user).await.unwrap(),
        Some(m.clone())
    );
    assert!(s.match_owned_by(&m.id, "someone").await.unwrap().is_none());
}

#[tokio::test]
async fn anonymous_match_unchanged() {
    let (s, _) = setup().await;
    let m = s.create_match(new_match(None), NOW).await.unwrap();
    assert!(m.owner_user_id.is_none());
    let by_scorer = s.by_scorer_token(&m.scorer_token).await.unwrap().unwrap();
    assert_eq!(by_scorer, m);
    // Retention still sees it as anonymous.
    let purged = s.purge_anonymous_idle_before(NOW + 1).await.unwrap();
    assert_eq!(purged, vec![m.id]);
}

#[tokio::test]
async fn live_matches_for_owner_newest_activity_first() {
    let (s, _) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    let old = s.create_match(new_match(Some(&user)), NOW).await.unwrap();
    let new = s
        .create_match(new_match(Some(&user)), NOW + 5)
        .await
        .unwrap();
    s.create_match(new_match(Some(&admin)), NOW + 9)
        .await
        .unwrap();
    s.create_match(new_match(None), NOW + 9).await.unwrap();
    let got: Vec<String> = s
        .live_matches_for_owner(&user)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(got, vec![new.id, old.id]);
}

#[tokio::test]
async fn purge_expired_auth_removes_only_expired_rows() {
    let (s, pool) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    let session = s.create_session(&user, NOW).await.unwrap();
    s.create_invite(&user, NOW).await.unwrap();
    s.create_reset(&user, &admin, NOW).await.unwrap();
    s.start_device("Watch", NOW).await.unwrap();
    let count = |table: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let none = s.purge_expired_auth(NOW).await.unwrap();
    assert_eq!(none.total(), 0);
    let later = NOW + SESSION_TTL_SECS;
    let purged = s.purge_expired_auth(later).await.unwrap();
    assert_eq!(
        (
            purged.sessions,
            purged.invites,
            purged.device_codes,
            purged.resets
        ),
        (1, 1, 1, 1)
    );
    for table in ["sessions", "invites", "device_codes", "password_resets"] {
        assert_eq!(count(table).await, 0, "{table}");
    }
    assert!(s.session_user(&session, NOW).await.unwrap().is_none());
    assert_eq!(count("users").await, 2);
}

#[tokio::test]
async fn approved_device_code_is_denied_after_reset() {
    let (s, _) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    // A reset in between approval and the poll.
    let started = s.start_device("Watch", NOW).await.unwrap();
    s.approve_device(&started.user_code, &user, NOW)
        .await
        .unwrap();
    let code = s.create_reset(&user, &admin, NOW).await.unwrap();
    s.consume_reset(&code, "h", NOW + 1).await.unwrap();
    assert_eq!(
        s.poll_device(&started.device_code, NOW + 2).await.unwrap(),
        PollOutcome::Denied
    );
    // The same for a password change.
    let started = s.start_device("Watch", NOW).await.unwrap();
    s.approve_device(&started.user_code, &user, NOW)
        .await
        .unwrap();
    s.set_password(&user, "h2", NOW + 1).await.unwrap();
    assert_eq!(
        s.poll_device(&started.device_code, NOW + 2).await.unwrap(),
        PollOutcome::Denied
    );
    // And for disabling then re-enabling.
    let started = s.start_device("Watch", NOW).await.unwrap();
    s.approve_device(&started.user_code, &user, NOW)
        .await
        .unwrap();
    s.set_disabled(&user, Some(NOW + 1)).await.unwrap();
    s.set_disabled(&user, None).await.unwrap();
    assert_eq!(
        s.poll_device(&started.device_code, NOW + 2).await.unwrap(),
        PollOutcome::Denied
    );
    assert!(s.list_devices(&user).await.unwrap().is_empty());
    // A code still pending (not approved) is left alone and can still be approved.
    let pending = s.start_device("Watch", NOW).await.unwrap();
    s.set_password(&user, "h3", NOW + 1).await.unwrap();
    s.approve_device(&pending.user_code, &user, NOW + 2)
        .await
        .unwrap();
}

#[tokio::test]
async fn debug_output_redacts_secrets() {
    let (s, _) = setup().await;
    let (_, user) = admin_and_user(&s).await;
    let invite = s.create_invite(&user, NOW).await.unwrap();
    let shown = format!("{invite:?}");
    assert!(!shown.contains(&invite.code), "{shown}");
    assert!(
        shown.contains("<redacted>") && shown.contains(&invite.id),
        "{shown}"
    );

    let started = s.start_device("Watch", NOW).await.unwrap();
    let shown = format!("{started:?}");
    assert!(!shown.contains(&started.device_code), "{shown}");
    assert!(
        shown.contains("<redacted>") && shown.contains(&started.user_code),
        "{shown}"
    );

    s.approve_device(&started.user_code, &user, NOW)
        .await
        .unwrap();
    let issued = s.poll_device(&started.device_code, NOW).await.unwrap();
    let PollOutcome::Issued { raw_token } = &issued else {
        panic!("expected a token");
    };
    let shown = format!("{issued:?}");
    assert!(!shown.contains(raw_token.as_str()), "{shown}");
    assert!(
        shown.contains("Issued") && shown.contains("<redacted>"),
        "{shown}"
    );
    assert_eq!(format!("{:?}", PollOutcome::Pending), "Pending");
}

#[tokio::test]
async fn using_one_reset_invalidates_others() {
    let (s, _) = setup().await;
    let (admin, user) = admin_and_user(&s).await;
    let first = s.create_reset(&user, &admin, NOW).await.unwrap();
    let second = s.create_reset(&user, &admin, NOW).await.unwrap();
    let other_user = s.create_reset(&admin, &admin, NOW).await.unwrap();
    s.consume_reset(&first, "h", NOW).await.unwrap();
    assert!(matches!(
        s.consume_reset(&second, "h", NOW).await,
        Err(StoreError::NotFound)
    ));
    // Another user's code is untouched.
    s.consume_reset(&other_user, "h", NOW).await.unwrap();
    // A password change invalidates outstanding codes, too.
    let third = s.create_reset(&user, &admin, NOW).await.unwrap();
    s.set_password(&user, "h2", NOW).await.unwrap();
    assert!(matches!(
        s.consume_reset(&third, "h", NOW).await,
        Err(StoreError::NotFound)
    ));
}
