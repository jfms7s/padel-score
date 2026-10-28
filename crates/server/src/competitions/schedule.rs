//! The date, place and sign-up settings of an event, and the rules that check them. Pure: no
//! database and no clock (`now` is a parameter), so it is tested on its own.

use serde::{Deserialize, Serialize};

use super::Kind;
use super::non_stop::{self, MAX_SCHEDULE_MATCHES, schedule_size};
use crate::store::{StoreError, normalize_place, present};

pub const DEFAULT_MIN_TEAMS: u32 = 4;
/// The bracket sizes a tournament can take.
pub const TOURNAMENT_SIZES: [u32; 4] = [4, 8, 16, 32];
/// Sign-ups close this long before the start, unless the start is nearer than that.
pub const DEADLINE_BEFORE_SECS: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signups {
    Off,
    /// Only people with the link can sign up.
    Private,
    /// The event is also listed for everyone.
    Public,
}

impl Signups {
    pub fn as_str(self) -> &'static str {
        match self {
            Signups::Off => "off",
            Signups::Private => "private",
            Signups::Public => "public",
        }
    }

    pub fn parse(s: &str) -> Result<Signups, sqlx::Error> {
        match s {
            "off" => Ok(Signups::Off),
            "private" => Ok(Signups::Private),
            "public" => Ok(Signups::Public),
            other => Err(sqlx::Error::Decode(
                format!("unknown sign-up mode {other:?}").into(),
            )),
        }
    }
}

/// What an event says about when and where it is and who can sign up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Schedule {
    pub starts_at: Option<i64>,
    pub place: Option<String>,
    pub signups: Signups,
    pub min_teams: u32,
    pub max_teams: Option<u32>,
    pub deadline_at: Option<i64>,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            starts_at: None,
            place: None,
            signups: Signups::Off,
            min_teams: DEFAULT_MIN_TEAMS,
            max_teams: None,
            deadline_at: None,
        }
    }
}

/// A requested change to a [`Schedule`]. An absent field keeps the current value; for the
/// `Option<Option<_>>` fields an explicit `null` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ScheduleInput {
    #[serde(default, deserialize_with = "present")]
    pub starts_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "present")]
    pub place: Option<Option<String>>,
    #[serde(default)]
    pub signups: Option<Signups>,
    #[serde(default)]
    pub min_teams: Option<u32>,
    #[serde(default, deserialize_with = "present")]
    pub max_teams: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    pub deadline_at: Option<Option<i64>>,
}

/// The largest even number of teams whose round robin, over `legs` legs, fits in
/// [`MAX_SCHEDULE_MATCHES`].
pub fn max_non_stop_teams(legs: u32) -> u32 {
    let legs = legs.max(1);
    let mut teams = 2;
    while schedule_size(teams as usize + 2, legs) <= MAX_SCHEDULE_MATCHES {
        teams += 2;
    }
    teams
}

/// The default sign-up deadline: 24h before the start, or the start itself when that is less
/// than 24h away.
fn default_deadline(starts_at: i64, now: i64) -> i64 {
    let day_before = starts_at.saturating_sub(DEADLINE_BEFORE_SECS);
    if day_before >= now {
        day_before
    } else {
        starts_at
    }
}

/// The schedule that results from applying `input` to `current`, or the invalid field. See the
/// field names in [`StoreError::InvalidField`]: `starts_at`, `place`, `signups`, `min_teams`,
/// `max_teams`, `deadline_at`.
pub fn normalize_schedule(
    kind: Kind,
    settings: &serde_json::Value,
    current: &Schedule,
    input: &ScheduleInput,
    now: i64,
) -> Result<Schedule, StoreError> {
    let starts_at = match input.starts_at {
        None => current.starts_at,
        Some(None) => None,
        Some(Some(at)) => {
            if current.starts_at != Some(at) && at <= now {
                return Err(StoreError::InvalidField("starts_at"));
            }
            Some(at)
        }
    };
    let place = match &input.place {
        None => current.place.clone(),
        Some(raw) => normalize_place(raw.as_deref())?,
    };
    let signups = input.signups.unwrap_or(current.signups);
    let min_teams = input.min_teams.unwrap_or(current.min_teams);
    let max_teams = input.max_teams.unwrap_or(current.max_teams);

    let turning_on = signups != Signups::Off && current.signups == Signups::Off;
    if signups != Signups::Off {
        match starts_at {
            None => return Err(StoreError::InvalidField("starts_at")),
            // Opening sign-ups for a start that has already passed.
            Some(start) if turning_on && start <= now => {
                return Err(StoreError::InvalidField("starts_at"));
            }
            Some(_) => {}
        }
        if max_teams.is_none() {
            return Err(StoreError::InvalidField("max_teams"));
        }
    }
    let min_floor = match kind {
        Kind::Tournament => 4,
        Kind::NonStop => 2,
    };
    if min_teams < min_floor {
        return Err(StoreError::InvalidField("min_teams"));
    }
    if let Some(max) = max_teams {
        let fits = match kind {
            Kind::Tournament => TOURNAMENT_SIZES.contains(&max),
            Kind::NonStop => {
                let legs = serde_json::from_value::<non_stop::Settings>(settings.clone())
                    .map_or(1, |s| s.legs);
                max.is_multiple_of(2) && max >= 2 && max <= max_non_stop_teams(legs)
            }
        };
        if !fits {
            return Err(StoreError::InvalidField("max_teams"));
        }
        if min_teams > max {
            return Err(StoreError::InvalidField("min_teams"));
        }
    }

    let deadline_at = match (starts_at, input.deadline_at) {
        (None, Some(Some(_))) => return Err(StoreError::InvalidField("deadline_at")),
        (None, _) => None,
        (Some(start), Some(Some(at))) => {
            // A stale deadline sent back unchanged is only kept while sign-ups stay as they were.
            let stale = at <= now && (turning_on || current.deadline_at != Some(at));
            if stale || at > start {
                return Err(StoreError::InvalidField("deadline_at"));
            }
            Some(at)
        }
        (Some(start), Some(None)) => Some(default_deadline(start, now)),
        (Some(start), None) => match current.deadline_at {
            // A kept deadline that has already passed is stale when sign-ups open.
            Some(kept) if current.starts_at == Some(start) && !(turning_on && kept <= now) => {
                Some(kept)
            }
            _ => Some(default_deadline(start, now)),
        },
    };
    Ok(Schedule {
        starts_at,
        place,
        signups,
        min_teams,
        max_teams,
        deadline_at,
    })
}

/// The total number of teams the event runs with when the organiser takes the sign-ups:
/// `hand_added` teams already made plus `complete` sign-ups, within `max_teams`, rounded down
/// to what the kind can run (a bracket size, or an even number), or 0 when too few.
pub fn take_count(kind: Kind, hand_added: usize, complete: usize, max_teams: u32) -> usize {
    let available = hand_added.saturating_add(complete).min(max_teams as usize);
    match kind {
        Kind::Tournament => TOURNAMENT_SIZES
            .iter()
            .map(|&size| size as usize)
            .filter(|&size| size <= available)
            .max()
            .unwrap_or(0),
        Kind::NonStop => {
            let even = available - available % 2;
            if even >= 2 { even } else { 0 }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    fn input(starts_at: i64) -> ScheduleInput {
        ScheduleInput {
            starts_at: Some(Some(starts_at)),
            ..ScheduleInput::default()
        }
    }

    fn normalize(
        kind: Kind,
        settings: &serde_json::Value,
        input: &ScheduleInput,
    ) -> Result<Schedule, StoreError> {
        normalize_schedule(kind, settings, &Schedule::default(), input, NOW)
    }

    fn invalid(res: Result<Schedule, StoreError>) -> &'static str {
        match res {
            Err(StoreError::InvalidField(f)) => f,
            other => panic!("expected InvalidField, got {other:?}"),
        }
    }

    #[test]
    fn deadline_defaults_to_24h_before_start() {
        let s = normalize(Kind::Tournament, &json!({}), &input(NOW + 3 * DAY)).unwrap();
        assert_eq!(s.deadline_at, Some(NOW + 2 * DAY));
        assert_eq!(s.signups, Signups::Off);
        assert_eq!(s.min_teams, 4);
    }

    #[test]
    fn deadline_defaults_to_start_when_start_is_within_24h() {
        let start = NOW + DAY / 2;
        let s = normalize(Kind::Tournament, &json!({}), &input(start)).unwrap();
        assert_eq!(s.deadline_at, Some(start));
        // Exactly 24h away: the 24h-before default is `now`, which is still allowed.
        let s = normalize(Kind::Tournament, &json!({}), &input(NOW + DAY)).unwrap();
        assert_eq!(s.deadline_at, Some(NOW));
    }

    #[test]
    fn deadline_after_start_is_refused() {
        let mut i = input(NOW + 3 * DAY);
        i.deadline_at = Some(Some(NOW + 3 * DAY + 1));
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &i)),
            "deadline_at"
        );
        i.deadline_at = Some(Some(NOW));
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &i)),
            "deadline_at"
        );
        i.deadline_at = Some(Some(NOW + 3 * DAY));
        assert!(normalize(Kind::Tournament, &json!({}), &i).is_ok());
    }

    #[test]
    fn start_must_be_in_the_future_and_place_is_normalized() {
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &input(NOW))),
            "starts_at"
        );
        let mut i = input(NOW + DAY);
        i.place = Some(Some("  Club  ".to_owned()));
        let s = normalize(Kind::Tournament, &json!({}), &i).unwrap();
        assert_eq!(s.place.as_deref(), Some("Club"));
        i.place = Some(Some("x".repeat(81)));
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &i)),
            "place"
        );
    }

    #[test]
    fn tournament_max_must_be_a_bracket_size() {
        let mut i = input(NOW + 3 * DAY);
        for ok in [4, 8, 16, 32] {
            i.max_teams = Some(Some(ok));
            assert!(normalize(Kind::Tournament, &json!({}), &i).is_ok(), "{ok}");
        }
        for bad in [0, 2, 6, 12, 64] {
            i.max_teams = Some(Some(bad));
            assert_eq!(
                invalid(normalize(Kind::Tournament, &json!({}), &i)),
                "max_teams",
                "{bad}"
            );
        }
        i.max_teams = Some(Some(8));
        i.min_teams = Some(3);
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &i)),
            "min_teams"
        );
    }

    #[test]
    fn non_stop_max_is_even_and_within_the_match_limit() {
        assert_eq!(max_non_stop_teams(1), 22);
        assert_eq!(max_non_stop_teams(2), 16);
        assert_eq!(max_non_stop_teams(3), 12);
        let mut i = input(NOW + 3 * DAY);
        i.min_teams = Some(2);
        i.max_teams = Some(Some(22));
        assert!(normalize(Kind::NonStop, &json!({ "legs": 1 }), &i).is_ok());
        assert_eq!(
            invalid(normalize(Kind::NonStop, &json!({ "legs": 2 }), &i)),
            "max_teams"
        );
        for bad in [1, 7, 24] {
            i.max_teams = Some(Some(bad));
            assert_eq!(
                invalid(normalize(Kind::NonStop, &json!({ "legs": 1 }), &i)),
                "max_teams",
                "{bad}"
            );
        }
        i.max_teams = Some(Some(2));
        assert!(normalize(Kind::NonStop, &json!({}), &i).is_ok());
        i.min_teams = Some(1);
        assert_eq!(
            invalid(normalize(Kind::NonStop, &json!({}), &i)),
            "min_teams"
        );
    }

    #[test]
    fn min_cannot_exceed_max() {
        let mut i = input(NOW + 3 * DAY);
        i.min_teams = Some(8);
        i.max_teams = Some(Some(4));
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &i)),
            "min_teams"
        );
    }

    #[test]
    fn signups_need_a_date_and_a_maximum() {
        let on = ScheduleInput {
            signups: Some(Signups::Public),
            ..ScheduleInput::default()
        };
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &on)),
            "starts_at"
        );
        let mut i = input(NOW + 3 * DAY);
        i.signups = Some(Signups::Private);
        assert_eq!(
            invalid(normalize(Kind::Tournament, &json!({}), &i)),
            "max_teams"
        );
        i.max_teams = Some(Some(8));
        let s = normalize(Kind::Tournament, &json!({}), &i).unwrap();
        assert_eq!((s.signups, s.max_teams), (Signups::Private, Some(8)));
        // Clearing the date while sign-ups are on is refused too.
        let off = ScheduleInput {
            starts_at: Some(None),
            ..ScheduleInput::default()
        };
        assert_eq!(
            invalid(normalize_schedule(
                Kind::Tournament,
                &json!({}),
                &s,
                &off,
                NOW
            )),
            "starts_at"
        );
    }

    #[test]
    fn deadline_is_recomputed_only_when_the_start_moves() {
        let mut i = input(NOW + 3 * DAY);
        i.deadline_at = Some(Some(NOW + DAY));
        let s = normalize(Kind::Tournament, &json!({}), &i).unwrap();
        assert_eq!(s.deadline_at, Some(NOW + DAY));
        let same = ScheduleInput {
            place: Some(Some("Club".to_owned())),
            ..ScheduleInput::default()
        };
        let kept = normalize_schedule(Kind::Tournament, &json!({}), &s, &same, NOW).unwrap();
        assert_eq!(kept.deadline_at, Some(NOW + DAY));
        let moved =
            normalize_schedule(Kind::Tournament, &json!({}), &s, &input(NOW + 5 * DAY), NOW)
                .unwrap();
        assert_eq!(moved.deadline_at, Some(NOW + 4 * DAY));
        // An explicit null asks for the default again.
        let reset = ScheduleInput {
            deadline_at: Some(None),
            ..ScheduleInput::default()
        };
        let back = normalize_schedule(Kind::Tournament, &json!({}), &s, &reset, NOW).unwrap();
        assert_eq!(back.deadline_at, Some(NOW + 2 * DAY));
    }

    #[test]
    fn opening_signups_refreshes_a_stale_deadline_and_refuses_a_past_start() {
        // Created with sign-ups off; the stored deadline has since passed.
        let created = normalize(Kind::Tournament, &json!({}), &input(NOW + 3 * DAY)).unwrap();
        let later = NOW + 2 * DAY + 3600;
        let on = ScheduleInput {
            signups: Some(Signups::Public),
            max_teams: Some(Some(8)),
            ..ScheduleInput::default()
        };
        let s = normalize_schedule(Kind::Tournament, &json!({}), &created, &on, later).unwrap();
        // Less than 24h to the start: the default is the start itself.
        assert_eq!(s.deadline_at, Some(NOW + 3 * DAY));
        // A still-valid deadline is kept.
        let s =
            normalize_schedule(Kind::Tournament, &json!({}), &created, &on, NOW + 3600).unwrap();
        assert_eq!(s.deadline_at, Some(NOW + 2 * DAY));
        // A start in the past is refused.
        let err = normalize_schedule(Kind::Tournament, &json!({}), &created, &on, NOW + 4 * DAY);
        assert_eq!(invalid(err), "starts_at");
    }

    #[test]
    fn opening_signups_refuses_the_stale_deadline_even_when_unchanged() {
        let created = normalize(Kind::Tournament, &json!({}), &input(NOW + 3 * DAY)).unwrap();
        let later = NOW + 2 * DAY + 3600;
        let stale = ScheduleInput {
            signups: Some(Signups::Public),
            max_teams: Some(Some(8)),
            deadline_at: Some(created.deadline_at),
            ..ScheduleInput::default()
        };
        let err = normalize_schedule(Kind::Tournament, &json!({}), &created, &stale, later);
        assert_eq!(invalid(err), "deadline_at");
        // With sign-ups already on, resending the saved deadline stays fine.
        let on = Schedule {
            signups: Signups::Public,
            max_teams: Some(8),
            ..created
        };
        let same = ScheduleInput {
            deadline_at: Some(on.deadline_at),
            ..ScheduleInput::default()
        };
        assert!(normalize_schedule(Kind::Tournament, &json!({}), &on, &same, later).is_ok());
    }

    #[test]
    fn zero_legs_does_not_hang() {
        assert_eq!(max_non_stop_teams(0), 22);
    }

    #[test]
    fn take_count_tournament() {
        assert_eq!(take_count(Kind::Tournament, 0, 11, 16), 8);
        assert_eq!(take_count(Kind::Tournament, 1, 4, 8), 4);
        assert_eq!(take_count(Kind::Tournament, 0, 3, 8), 0);
        assert_eq!(take_count(Kind::Tournament, 0, 40, 16), 16);
    }

    #[test]
    fn take_count_non_stop() {
        assert_eq!(take_count(Kind::NonStop, 0, 7, 10), 6);
        assert_eq!(take_count(Kind::NonStop, 2, 9, 8), 8);
        assert_eq!(take_count(Kind::NonStop, 0, 1, 8), 0);
    }
}
