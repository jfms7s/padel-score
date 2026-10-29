// TS mirrors of the JSON that serde writes for the engine (crates/scoring) and the server
// (crates/server). Every union here was settled from a real serialized sample. The samples live
// in ./fixtures, and fixtures.test.ts checks each one against these shapes at runtime through
// ./guards (strict: no missing, extra or mistyped field), so keep the two files in step.

// ---- engine: rules and events ------------------------------------------------------------

export type Team = 'a' | 'b';

/** A place on court: the team and which of its two players (the engine's `Player`). */
export interface CourtPlayer {
	team: Team;
	/** 0 or 1. */
	index: number;
}

export type DeuceRule = 'advantage' | 'golden_point' | 'star_point';
export type SetsFormat = 'one' | 'best_of_3';
export type DecidingSet = 'full_set' | 'super_tie_break';

export interface SetRules {
	deuce: DeuceRule;
	sets: SetsFormat;
	deciding: DecidingSet;
}

/** Externally tagged: `{"sets":{..}}` or `{"timed":{..}}`. */
export type Format = { sets: SetRules } | { timed: { deuce: DeuceRule; draws_allowed: boolean } };

export interface Start {
	first_server: CourtPlayer;
	/** The index (0 or 1) of the player who serves first for the team that receives first. */
	other_first_server: number;
}

/** `{"point":"a"}` or the bare string `"time_up"`. */
export type Event = { point: Team } | 'time_up';

export type RulesError = 'super_tie_break_needs_multiple_sets' | 'invalid_player_index';

/** Unit variants are strings; `InvalidRules` is `{"invalid_rules": RulesError}`. */
export type ErrorKind = 'match_over' | 'not_timed' | 'already_time_up' | { invalid_rules: RulesError };

// ---- engine: score -----------------------------------------------------------------------

export type PointCall = 'love' | 'fifteen' | 'thirty' | 'forty' | 'advantage';

export interface GameScore {
	a: PointCall;
	b: PointCall;
	/** How many times this game has reached 40-40. */
	deuces: number;
	/** Level at the deuce the golden or star point decides: the next point wins the game. */
	deciding_point: boolean;
}

export interface TieBreakScore {
	a: number;
	b: number;
	super_tiebreak: boolean;
}

/** `{"game":..}`, `{"tie_break":..}`, or `"none"` once the match is over. */
export type Current = { game: GameScore } | { tie_break: TieBreakScore } | 'none';

/** `winner` is null only for a timed draw. */
export type Status = 'in_progress' | { finished: { winner: Team | null } };

export type ServeSide = 'right' | 'left' | 'receiver_chooses';

export interface SetScore {
	a: number;
	b: number;
	/** Tie-break points, `[a, b]`, for a set that went to one. */
	tiebreak: [number, number] | null;
}

export interface Score {
	/** The finished sets, then the one in progress. Empty for a timed match. */
	sets: SetScore[];
	sets_won: [number, number];
	/** Games in the current set; for a timed match, the whole match. */
	games: [number, number];
	regulation_games: [number, number];
	current: Current;
	/** Who serves the next point. Meaningless once finished. */
	server: CourtPlayer;
	/** Where the next serve goes from. Meaningless once finished. */
	side: ServeSide;
	/** True only in the snapshot right after the event that made the players change ends. */
	change_ends: boolean;
	overtime: boolean;
	time_up: boolean;
	status: Status;
}

// ---- server ------------------------------------------------------------------------------

export interface Teams {
	a: [string, string];
	b: [string, string];
}

export interface ViewEvent {
	seq: number;
	event: Event;
	/** When the point was recorded, Unix seconds. */
	at: number;
}

export interface MatchView {
	id: string;
	format: Format;
	start: Start;
	teams: Teams;
	/** Bumped by every applied command (append, undo, replace). */
	revision: number;
	/** The highest seq of any row, effective or not; 0 for an empty log. */
	head_seq: number;
	/** The effective events only, in seq order. */
	events: ViewEvent[];
	score: Score;
	/** The coordinator typed the result: scoring links can't change it. Present only when true. */
	locked?: boolean;
}

/** Internally tagged on `kind`. */
export type Command = { kind: 'point'; team: Team } | { kind: 'time_up' } | { kind: 'undo'; target_seq: number };

export interface CommandRequest {
	client_id: string;
	/** A batch (queued offline) must carry the current revision; a live command need not. */
	batch: boolean;
	base_revision: number;
	commands: Command[];
}

/** "Keep mine" after a conflict: drop the server's events after `base_seq`, then apply mine. */
export interface ReplaceRequest {
	client_id: string;
	base_revision: number;
	base_seq: number;
	commands: Command[];
}

export interface CreateMatchRequest {
	format: Format;
	start: Start;
	teams: Teams;
	/** Signed in only: the slots picked from the player list. Left out when none is picked. */
	players?: PlayerSlots;
	/** Signed in only: what the match is started from. It marks that source played. */
	from?: StartedFrom;
}

/** What a match is started from (crates/server/src/starting.rs `StartedFrom`). */
export type StartedFrom = { kind: 'game'; id: string } | { kind: 'scheduled'; id: string };

/** The four players to fill New match with, in seat order (crates/server/src/starting.rs). */
export interface StartPlayers {
	players: { player_id: string; name: string }[];
	/** Null for a board game: New match keeps its last setup. */
	format: Format | null;
	from: StartedFrom;
}

export interface CreatedMatch {
	id: string;
	scorer_token: string;
	spectator_token: string;
}

/**
 * What a push carries (crates/server/src/push.rs `Payload`): a kind, a same-origin path and
 * the fields that kind uses. The kind is open: a newer server may send one this build lacks.
 */
export interface NoticePayload {
	kind: string;
	url?: string;
	title?: string;
	start?: number;
	end?: number;
	new_start?: number;
	new_end?: number;
	name?: string;
	accepted?: boolean;
}

/** Every non-2xx body the API sends has an `error` code. */
export type ApiErrorBody =
	| { error: 'conflict'; view: MatchView }
	| { error: 'locked'; view: MatchView }
	| { error: 'invalid_score'; reason: InvalidScore }
	| { error: 'rejected'; kind: ErrorKind; index: number }
	| { error: 'invalid'; message: string }
	| { error: 'invalid_match'; message: string }
	| {
			error: 'not_found' | 'bad_request' | 'method_not_allowed' | 'payload_too_large' | 'internal';
	  }
	| { error: 'validation'; field: string }
	| { error: 'finished' | 'event_match' }
	| { error: 'too_many_requests'; retry_after_secs: number }
	| { error: AccountErrorCode }
	| { error: EventErrorCode }
	| { error: SignupErrorCode }
	| { error: BoardErrorCode }
	| { error: ScheduledErrorCode };

// ---- accounts (crates/server/src/accounts.rs, owned.rs, device.rs) ------------------------

/** The account routes' other error codes (401, 403, 409, 410). */
export type AccountErrorCode =
	| 'unauthorized'
	| 'invalid_credentials'
	| 'forbidden'
	| 'setup_done'
	| 'email_taken'
	| 'too_many_invites'
	| 'invite_used'
	| 'invite_invalid'
	| 'reset_invalid'
	| 'cannot_disable_self'
	| 'last_admin'
	| 'device_code_used'
	| 'invalid_player'
	| 'player_exists'
	| LinkingErrorCode;

/** `GET /api/auth/me`, and the answer to setup, login and accepting an invite. */
export interface Me {
	id: string;
	email: string;
	display_name: string;
	is_admin: boolean;
}

export interface SetupStatus {
	needs_setup: boolean;
}

export interface SetupRequest {
	setup_code: string;
	email: string;
	display_name: string;
	password: string;
}

export interface AcceptInviteRequest {
	email: string;
	display_name: string;
	password: string;
}

/** `POST /api/invites`: the code is only ever in the url's fragment (`/invite#<code>`). */
export interface CreatedInvite {
	id: string;
	url: string;
	expires_at: number;
}

export type InviteStatus = 'active' | 'used' | 'expired' | 'revoked';

/** One of `GET /api/invites`. Times are Unix seconds. */
export interface Invite {
	id: string;
	created_at: number;
	expires_at: number;
	status: InviteStatus;
	used_by_name?: string;
}

/** `POST /api/invites/lookup` with `{code}`. */
export interface InviteLookup {
	invited_by_name: string;
	expires_at: number;
}

/** One of `GET /api/admin/users`. */
export interface AdminUser {
	id: string;
	email: string;
	display_name: string;
	is_admin: boolean;
	disabled: boolean;
	invited_by_name?: string;
	created_at: number;
}

/** `POST /api/admin/users/{id}/reset-link`: `/reset#<code>`, shown once. */
export interface ResetLink {
	url: string;
	expires_at: number;
}

/** `GET /api/device/{user_code}`: the watch waiting for approval. */
export interface DeviceLookup {
	device_name: string;
	created_secs_ago: number;
	expires_in_secs: number;
}

/** One of `GET /api/me/devices`: a signed-in watch. */
export interface Device {
	id: string;
	name: string;
	/** The token's first characters, to tell two watches apart. */
	prefix: string;
	created_at: number;
	last_used_at?: number | null;
}

/** One of `GET /api/me/matches`: a live match the user started. The tokens come to sessions only. */
export interface MyMatch {
	id: string;
	label: string;
	created_at: number;
	last_activity_at: number;
	view: MatchView;
	scorer_token?: string;
	spectator_token?: string;
	/** An event match on a court: where it is, e.g. "Court 1 · Sunday cup". */
	context?: string;
}

// ---- players and history (crates/server/src/players_api.rs, results.rs) -----------------

/** The linking routes' error codes (403, 409, 410, 429). */
export type LinkingErrorCode =
	| 'link_code_invalid'
	| 'link_overlaps'
	| 'player_linked'
	| 'player_not_linked'
	| 'already_yours'
	| 'claim_decided'
	| 'claim_declined'
	| 'not_public'
	| 'too_many_claims'
	| 'history_private';

/** A player's wins, losses and draws (timed draws) over finished matches. */
export interface PlayerRecord {
	wins: number;
	losses: number;
	draws: number;
}

/** One of the coordinator's players. Never deleted, only archived. */
export interface Player {
	id: string;
	name: string;
	archived: boolean;
	created_at: number;
	record: PlayerRecord;
	/** The account this entry is linked to, if any. */
	link: PlayerLink | null;
	/** Pending claims on the entry. */
	pending_claims: number;
}

/** `PATCH /api/players/{id}`: either field, or both. */
export interface PlayerChange {
	name?: string;
	archived?: boolean;
}

/** The player picked for each slot of a match, team A then team B; null = typed name only. */
export interface PlayerSlots {
	a: [string | null, string | null];
	b: [string | null, string | null];
}

/** A finished match's score as history keeps it. */
export interface FinalScore {
	/** The sets in order; empty for a timed match. */
	sets: SetScore[];
	/** The final set's games; for a timed match, the whole match including any overtime game. */
	games: [number, number];
	/** The games before any overtime game. */
	regulation_games: [number, number];
	overtime: boolean;
	timed: boolean;
}

/** One of `GET /api/me/history`: a finished match the user started. */
export interface HistoryEntry {
	match_id: string;
	finished_at: number;
	/** The names as played. */
	teams: Teams;
	players: PlayerSlots;
	/** Null for a timed draw. */
	winner: Team | null;
	score: FinalScore;
	/** The final view is at `/v/{spectator_token}`. */
	spectator_token: string;
	/** Event matches only: the event it was played in. */
	event?: EventContext;
}

/** Where an event match in history was played. */
export interface EventContext {
	id: string;
	name: string;
	kind: EventKind;
	label: string;
}

export type Outcome = 'win' | 'loss' | 'draw';

/** One of a player page's matches: which side the player was on and how it went. */
export interface PlayerHistoryEntry extends HistoryEntry {
	side: Team;
	outcome: Outcome;
}

/** A place in a tournament or non-stop event (phases 6-8). */
export interface Placing {
	event_id: string;
	event_name: string;
	event_kind: EventKind;
	place: number;
	/** Equal to `place` unless the place is shared (3rd-4th). */
	place_to: number;
	field_size: number;
	finished_at: number;
}

/** A page of a newest-first list; `next` is the `before` of the following page, null on the last. */
export interface Page<T> {
	items: T[];
	next: string | null;
}

/** `GET /api/players/{id}`: paging applies to `items` only. */
export interface PlayerPage extends Page<PlayerHistoryEntry> {
	player: Player;
	placings: Placing[];
}

// ---- events (crates/server/src/events_api.rs, court_api.rs, competitions/) --------------

/** The events routes' error codes (409, 410, 422 `invalid_score`). */
export type EventErrorCode =
	| 'already_started'
	| 'event_started'
	| 'event_not_running'
	| 'event_finished'
	| 'too_few_teams'
	| 'no_courts'
	| 'odd_teams'
	| 'schedule_too_big'
	| 'too_many_teams'
	| 'too_many_courts'
	| 'player_in_team'
	| 'court_in_use'
	| 'match_not_finished'
	| 'match_finished'
	| 'event_cancelled';

/**
 * A final score typed in after the fact (the engine's `FinalScore`). A 7-6 set carries its
 * tie-break points; a deciding super tie-break is `1-0` or `0-1` with its points in `tiebreak`.
 * `a` and `b` of a timed score are the regulation games; `overtime_winner` only when level.
 */
export type TypedScore = { sets: SetScore[] } | { timed: { a: number; b: number; overtime_winner: Team | null } };

/** Why the engine refuses a typed score. Unit variants are strings, the others one-key objects. */
export type InvalidScore =
	| { invalid_rules: RulesError }
	| 'wrong_format'
	| { impossible_set: { index: number } }
	| { missing_tie_break: { index: number } }
	| 'match_not_finished'
	| 'extra_sets'
	| 'draw_not_allowed'
	| 'overtime_winner_when_not_level';

/** The engine's `validate_final`: the winner (null for a draw), or why the score is refused. */
export type ValidateFinalResult = { winner: Team | null } | { error: InvalidScore };

/** `POST /api/score/{token}/final`: a court phone's typed result. */
export interface FinalRequest {
	client_id: string;
	base_revision: number;
	score: TypedScore;
}

/** `POST /api/events/{id}/matches/{match_id}/result`: the coordinator's, which locks the match. */
export interface EventResultRequest {
	client_id: string;
	score: TypedScore;
}

export type EventKind = 'tournament' | 'non_stop';
/** A cancelled event was called off before it started. */
export type EventStatus = 'setup' | 'running' | 'finished' | 'cancelled';

export type SignupMode = 'off' | 'private' | 'public';

/** When and where an event is and who can sign up. Times are Unix seconds. */
export interface Schedule {
	starts_at: number | null;
	place: string | null;
	signups: SignupMode;
	min_teams: number;
	max_teams: number | null;
	deadline_at: number | null;
}

/**
 * A change to a schedule, in `schedule` of `POST /api/events` and `PATCH /api/events/{id}`.
 * Leave a field out to keep it; `null` clears a date, place or maximum, and a `null` deadline
 * asks for the default one.
 */
export interface ScheduleInput {
	starts_at?: number | null;
	place?: string | null;
	signups?: SignupMode;
	min_teams?: number;
	max_teams?: number | null;
	deadline_at?: number | null;
}

/** An event as `GET /api/events` lists it, and inside every detail. Times are Unix seconds. */
export interface EventSummary {
	id: string;
	name: string;
	kind: EventKind;
	/** A tournament plays sets, a non-stop is timed. */
	format: Format;
	/** Owned by the kind (phases 7 and 8); `{}` in this phase. */
	settings: Record<string, unknown>;
	status: EventStatus;
	created_at: number;
	started_at: number | null;
	finished_at: number | null;
	team_count: number;
	court_count: number;
	schedule: Schedule;
	cancelled_at: number | null;
	/** When sign-ups closed: the deadline ran, or the organiser took the teams. */
	signups_closed_at: number | null;
}

export interface TeamPlayer {
	id: string;
	name: string;
}

export interface EventTeam {
	id: string;
	/** The shown name: the custom name, or else "Ana / Marta" from the players' names. */
	name: string;
	custom_name: string | null;
	players: [TeamPlayer, TeamPlayer];
}

export type EventMatchState = 'waiting' | 'live' | 'finished';

/** An event's match as the coordinator sees it. */
export interface EventMatch {
	/** The match id. */
	id: string;
	label: string;
	/** Creation order within the event. */
	ordinal: number;
	/** Team ids. */
	team_a: string;
	team_b: string;
	court_id: string | null;
	state: EventMatchState;
	/** The coordinator typed the result. */
	locked: boolean;
	winner: Team | null;
	result: FinalScore | null;
	/** The score so far, for a live match only. */
	live: Score | null;
	scorer_token: string;
	spectator_token: string;
}

/** A court as the coordinator sees it. */
export interface CourtOut {
	id: string;
	name: string;
	/** 1-based, in creation order. */
	position: number;
	/** The court phone's link is `/c/{scorer_token}`. */
	scorer_token: string;
	/** The court's viewing link is `/cv/{spectator_token}`. */
	spectator_token: string;
	/** Court phones connected right now. */
	phones: number;
	/** The match on court (a finished one stays until the court moves on). */
	current: string | null;
	/** Match ids waiting for this court, in order. */
	queue: string[];
}

/** A team's final position; a shared place (3rd-4th) has `place_to > place`. */
export interface Place {
	team_id: string;
	place: number;
	place_to: number;
}

/** `GET /api/events/{id}`, and each `event` frame of its stream. */
export interface EventDetail {
	event: EventSummary;
	/** The whole event's viewing link is `/e/{spectator_token}`. */
	spectator_token: string;
	/** The join link is `/j/{join_token}`, once sign-ups were turned on. */
	join_token: string | null;
	teams: EventTeam[];
	courts: CourtOut[];
	matches: EventMatch[];
	/** Filled once the event is finished. */
	places: Place[];
	/** The kind works out the places itself, so Finish sends none (false in this phase). */
	places_computed: boolean;
	/** Owned by the kind (phases 7 and 8); null in this phase. */
	kind_data: unknown;
}

/** A match on a court, as the court phone sees it. */
export interface CourtMatch {
	id: string;
	label: string;
	teams: Teams;
	spectator_token: string;
	/** Only for the court's scoring link. */
	scorer_token?: string;
}

/** `GET /api/court/{token}`, and each `court` frame of its stream. */
export interface CourtState {
	event: { id: string; name: string; kind: EventKind; status: EventStatus };
	court: { id: string; name: string };
	current: CourtMatch | null;
	/** The next match queued for this court. */
	next: CourtMatch | null;
	kind_data: unknown;
}

/** An event match as a spectator sees it: no scoring link. */
export type EventViewMatch = Omit<EventMatch, 'scorer_token'>;

/** A court as a spectator sees it: no scoring link and no phone count. */
export type EventViewCourt = Omit<CourtOut, 'scorer_token' | 'phones'>;

/** `GET /api/event-view/{token}`, and each `event` frame of its stream. */
export interface EventView extends Omit<EventDetail, 'courts' | 'matches' | 'join_token'> {
	courts: EventViewCourt[];
	matches: EventViewMatch[];
}

/** `POST /api/events`. */
export interface NewEvent {
	name: string;
	kind: EventKind;
	format: Format;
	settings?: Record<string, unknown>;
	schedule?: ScheduleInput;
}

/** `PATCH /api/events/{id}`: any of the fields. */
export interface EventChange {
	name?: string;
	format?: Format;
	settings?: Record<string, unknown>;
	schedule?: ScheduleInput;
}

/** `POST /api/events/{id}/teams`: two player ids, and a custom name if wanted. */
export interface NewTeam {
	players: [string, string];
	name?: string;
}

/** `PATCH /api/events/{id}/teams/{team_id}`: `name: null` clears the custom name. */
export interface TeamChange {
	name?: string | null;
	players?: [string, string];
}

/** `POST /api/events/{id}/matches`. */
export interface NewEventMatch {
	team_a: string;
	team_b: string;
	label?: string;
	court_id?: string;
}

/** `PATCH /api/events/{id}/matches/{match_id}`: assign, move or take off a court (null). */
export interface MatchChange {
	court_id: string | null;
	label?: string;
}

// ---- linking, claims and privacy (crates/server/src/linking_api.rs) ----------------------

/** `Player.link`: who an entry is linked to. */
export interface PlayerLink {
	display_name: string;
	linked_at: number;
	is_me: boolean;
	/** Linked by a game or sign-up and not yet approved: their history stays private. */
	automatic: boolean;
}

/** All a claimant or link-code holder sees of an entry before it is linked. */
export interface ClaimPreview {
	name: string;
	coordinator_name: string;
	matches: number;
	events: number;
	first_at: number | null;
	last_at: number | null;
}

export type SlotStateName = 'claimable' | 'yours' | 'pending' | 'linked' | 'declined';

/** One picked slot of a match: `GET /api/view/{token}/slots`. */
export interface SlotState {
	team: Team;
	position: number;
	name: string;
	state: SlotStateName;
}

/** One player of an event's team (position 0 or 1): `GET /api/event-view/{token}/slots`. */
export interface EventSlotState {
	team_id: string;
	position: number;
	name: string;
	state: SlotStateName;
}

/** What a claim points at: a match's picked slot, or a player of an event's team. */
export type ClaimTarget =
	| { kind: 'match_slot'; spectator_token: string; team: Team; position: number }
	| { kind: 'event_team_slot'; event_view_token: string; team_id: string; position: number };

/** A claim as the coordinator sees it: `GET /api/claims`. */
export interface Claim {
	id: string;
	player: { id: string; name: string };
	claimant: { display_name: string; email: string };
	preview: ClaimPreview;
	created_at: number;
}

/** A claim as its claimant sees it: `GET /api/me/claims`. */
export interface MyClaim {
	id: string;
	status: 'pending' | 'approved' | 'declined' | 'withdrawn';
	preview: ClaimPreview;
	created_at: number;
	decided_at: number | null;
}

/** An entry linked to the signed-in account: `GET /api/me/links`. */
export interface MyLink {
	player_id: string;
	name: string;
	coordinator_name: string;
	linked_at: number;
}

/** A coordinator's link code: the link to show as a QR code, and when it stops working. */
export interface CreatedLinkCode {
	url: string;
	expires_at: number;
	/** Seconds left by the server's clock; count down from this, not from `expires_at`. */
	expires_in: number;
}

/**
 * One match of a combined history; the redacted form has no names, coordinator or link, and of
 * an event match's event only the kind.
 */
export type LinkedEntry =
	| {
			match_id: string;
			finished_at: number;
			outcome: Outcome;
			side: Team;
			score: FinalScore;
			redacted: false;
			teams: Record<Team, string[]>;
			coordinator_name: string;
			spectator_token: string;
			/** Event matches only. */
			event?: EventContext;
	  }
	| {
			match_id: string;
			finished_at: number;
			outcome: Outcome;
			side: Team;
			score: FinalScore;
			redacted: true;
			/** Event matches only: the kind, nothing else. */
			event?: { kind: EventKind };
	  };

/** A placing in a combined history: the event's name only where the viewer may see it. */
export type LinkedPlacing = Omit<Placing, 'event_name'> & { event_name?: string };

/** `GET /api/me/results`, `/api/players/{id}/linked-history` and `/api/profiles/{token}`. */
export interface LinkedHistory extends Page<LinkedEntry> {
	display_name: string;
	record: PlayerRecord;
	placings: LinkedPlacing[];
}

export type Visibility = 'private' | 'coordinators' | 'public';

/** `GET` and `PUT /api/me/privacy`. */
export interface Privacy {
	visibility: Visibility;
	profile_url: string | null;
}

// ---- WASM bridge -------------------------------------------------------------------------

/** What the engine's `replay` reports: an event it refused, or input it could not parse. */
export type EngineError = { kind: ErrorKind; index: number } | { kind: 'bad_json'; message: string };

export type ReplayResult = { ok: Score } | { err: EngineError };

// ---- game boards (crates/server/src/boards/mod.rs, boards_api.rs) -------------------------

/** The board routes' 409 codes (the 422 fields are `range`, `title`, `place`, `time_zone`, `week`). */
export type BoardErrorCode =
	| 'board_over'
	| 'board_full'
	| 'too_many_boards'
	| 'creator_only'
	| 'too_many_ranges'
	| 'guests_off'
	| 'guest_name_taken'
	| GameErrorCode;

/** The game actions' 409 codes (crates/server/src/boards/actions.rs, starting.rs). */
export type GameErrorCode =
	| 'game_changed'
	| 'nobody_can_take'
	| 'too_late'
	| 'cannot_swap'
	| 'players_busy'
	| 'too_soon'
	| 'request_pending'
	| 'request_expired'
	| 'already_answered'
	| 'not_yet'
	| 'game_over'
	| 'already_started'
	| 'not_in_game'
	| 'game_played'
	| 'game_started'
	| 'already_in_game'
	| 'game_full'
	| 'busy';

export interface NewBoard {
	title: string;
	place: string | null;
	time_zone: string;
	starts_at: number;
	ends_at: number;
	allow_guests?: boolean;
}

/** `place: null` clears it; leave a field out to keep it. */
export interface BoardChange {
	title?: string;
	place?: string | null;
	allow_guests?: boolean;
}

export type GameState = 'full' | 'short' | 'played' | 'past';

export interface BoardOut {
	id: string;
	token: string;
	title: string;
	place: string | null;
	time_zone: string;
	starts_at: number;
	ends_at: number;
	creator_user_id: string;
	creator_name: string;
	over: boolean;
	allow_guests: boolean;
}

export interface MemberOut {
	user_id: string;
	name: string;
	guest: boolean;
	joined_at: number;
}

/** A piece of a member's free time; touching pieces of one member are drawn as one range. */
export interface PieceOut {
	user_id: string;
	start: number;
	end: number;
	created_at: number;
}

export interface MemberRef {
	user_id: string;
	name: string;
	guest: boolean;
}

export interface GameOut {
	id: string;
	start: number;
	end: number;
	version: number;
	moved: boolean;
	state: GameState;
	/** In seat order. */
	players: MemberRef[];
	/** User ids next in line for a place. */
	line: string[];
	match_spectator_token: string | null;
}

export interface RequestOut {
	id: string;
	kind: 'swap' | 'time';
	from: MemberRef;
	game_id: string;
	other_game_id: string | null;
	new_start: number | null;
	new_end: number | null;
	needs_my_answer: boolean;
}

/** `null` in the view when signed out. */
export interface MeOut {
	user_id: string;
	member: boolean;
	creator: boolean;
	guest: boolean;
}

export interface BoardView {
	board: BoardOut;
	me: MeOut | null;
	members: MemberOut[];
	pieces: PieceOut[];
	games: GameOut[];
	requests: RequestOut[];
}

/** The answer to a guest joining or picking a name: the key to keep, and the board as it now is. */
export interface GuestJoined {
	guest_key: string;
	view: BoardView;
}

// ---- event sign-ups (crates/server/src/competitions/signups.rs, signups_api.rs) -----------

/** The sign-up routes' 409 codes (the 422 fields are the schedule's and `partner_name`). */
export type SignupErrorCode =
	| 'event_cancelled'
	| 'signups_closed'
	| 'signups_full'
	| 'signups_off'
	| 'signups_exist'
	| 'own_link'
	| 'already_signed_up'
	| 'not_solo'
	| 'signup_taken'
	| 'not_auto_paired'
	| 'nothing_to_take';

/** Where a user stands in an event. Places are 1-based. */
export type MyEntry =
	| { state: 'solo' }
	| { state: 'pending'; partner_token: string }
	| { state: 'team'; place: number }
	| { state: 'line'; place: number };

export interface SignupPlayer {
	/** Left out of the public join page. */
	user_id?: string | null;
	name: string;
}

export interface SignupTeam {
	id: string;
	players: SignupPlayer[];
	complete_at: number | null;
	auto_paired: boolean;
	held: boolean;
	partner_pending: boolean;
}

export interface SignupList {
	/** Teams the organiser made by hand: they hold a place of their own. */
	hand_added: number;
	teams: SignupTeam[];
	line: SignupTeam[];
	/** People waiting for a partner, held ones included. */
	solos: SignupTeam[];
	/** People who sent a partner link that is not answered yet. */
	pending: SignupTeam[];
}

/** What the join page shows about the event. */
export interface EventJoinInfo {
	name: string;
	kind: EventKind;
	format: Format;
	schedule: Schedule;
	status: EventStatus;
	organiser_name: string;
	/** Nobody can sign up any more: cancelled, started, past the deadline or taken. */
	closed: boolean;
}

/** What a join link opens for an event. */
export type EventJoinView = { kind: 'event'; event: EventJoinInfo; list: SignupList; me: MyEntry | null };

/** `GET /api/join/{token}`: an event, else a scheduled match. */
export type JoinView = EventJoinView | { kind: 'match'; view: ScheduledView };

// ---- scheduled matches (crates/server/src/scheduled.rs) -----------------------------------

export type Listing = 'private' | 'public';

export type ScheduledState = 'open' | 'full' | 'started' | 'cancelled' | 'past';

/** A place (its slot, 0-3) or a spot in the line (its position). `user_id` is the owner's or yours. */
export interface PlaceOut {
	slot: number;
	/** Null for an open place. */
	name: string | null;
	user_id: string | null;
}

export interface ScheduledMe {
	owner: boolean;
	/** The place you hold. */
	slot: number | null;
	in_line: boolean;
}

export interface ScheduledView {
	id: string;
	join_token: string;
	owner_name: string;
	starts_at: number;
	place: string | null;
	listing: Listing;
	format: Format;
	/** Always four, in slot order. */
	places: PlaceOut[];
	line: PlaceOut[];
	state: ScheduledState;
	/** Set once the match was started: where to watch it. */
	match_spectator_token: string | null;
	/** Null when signed out. */
	me: ScheduledMe | null;
}

/** `POST /api/scheduled`: each place is one of your player ids, or null (open). */
export interface NewScheduled {
	starts_at: number;
	place?: string;
	listing: Listing;
	format: Format;
	places: [string | null, string | null, string | null, string | null];
}

/** `PATCH /api/scheduled/{id}`: what is absent stays; `place: null` clears it. */
export interface ScheduledChange {
	starts_at?: number;
	place?: string | null;
}

/** The scheduled routes' 409 codes (the 422 fields are `starts_at` and `place`). */
export type ScheduledErrorCode =
	| 'already_in'
	| 'line_full'
	| 'match_closed'
	| 'match_short'
	| 'same_player_twice'
	| 'too_many_scheduled'
	| 'not_yet'
	| 'game_over'
	| 'already_started';

/** `POST /api/join/{token}`. */
export type JoinAs = { as: 'solo' } | { as: 'partner'; partner_name?: string };

/** `GET /api/partner/{ptoken}`. */
export interface PartnerInvite {
	event_name: string;
	inviter_name: string;
	join_token: string;
}

/** What "Take teams from sign-ups" did. */
export interface TakeReport {
	taken: number;
	/** Sign-up ids left out because a player is already in a hand-added team. */
	skipped: string[];
	left_on_line: number;
}

// ---- the calendar (crates/server/src/calendar.rs) -----------------------------------------

export type CalendarRole = 'organiser' | 'player' | 'line';

export type CalendarItem =
	| {
			kind: 'game';
			board_token: string;
			board_title: string;
			game_id: string;
			start: number;
			end: number;
			players: string[];
			state: GameState;
			/** Where the board plays, if it says. */
			board_place: string | null;
			/** The caller's index in `players`. */
			me: number;
	  }
	| {
			kind: 'event';
			/** Set for the organiser only. */
			event_id: string | null;
			join_token: string | null;
			name: string;
			event_kind: EventKind;
			starts_at: number;
			place: string | null;
			role: CalendarRole;
			team: string[] | null;
			place_in_line: number | null;
			status: EventStatus;
			/** Teams signed up, and the cap if there is one. */
			teams: number;
			max_teams: number | null;
	  }
	| {
			kind: 'match';
			join_token: string;
			starts_at: number;
			place: string | null;
			role: CalendarRole;
			/** The four places in order; null is an open place. */
			players: (string | null)[];
			state: ScheduledState;
			owner_name: string;
	  };

export type WaitingItem =
	| {
			kind: 'swap';
			request_id: string;
			board_token: string;
			from_name: string;
			/** [start, end] of my game. */
			my_game: [number, number];
			/** [start, end] of the asker's game. */
			their_game: [number, number];
	  }
	| {
			kind: 'time';
			request_id: string;
			board_token: string;
			from_name: string;
			game: [number, number];
			new: [number, number];
	  }
	| { kind: 'partner'; join_token: string; event_name: string; partner_token: string };

export interface BoardStrip {
	token: string;
	title: string;
	starts_at: number;
	ends_at: number;
	/** My games on it that have not ended. */
	my_games: number;
}

export type OpenItem =
	| {
			kind: 'event';
			join_token: string;
			name: string;
			event_kind: EventKind;
			starts_at: number;
			place: string | null;
			teams: number;
			max_teams: number;
	  }
	| {
			kind: 'match';
			join_token: string;
			owner_name: string;
			starts_at: number;
			place: string | null;
			open_places: number;
	  };

export interface Calendar {
	waiting: WaitingItem[];
	items: CalendarItem[];
	boards: BoardStrip[];
	open: OpenItem[];
}

/** An event on the Organise page: the summary plus what needs the organiser. */
export interface OrganisedEvent extends EventSummary {
	join_token: string | null;
	/** Teams with a place (`team_count` counts only hand-added and taken teams). */
	signed_teams: number;
	solo_count: number;
	line_count: number;
}

/** A scheduled match you own. `places` holds the name in each place by slot; null is open. */
export interface OrganisedMatch {
	id: string;
	join_token: string;
	starts_at: number;
	place: string | null;
	state: ScheduledState;
	places: [string | null, string | null, string | null, string | null];
	line_count: number;
}

export interface OrganisedBoard {
	token: string;
	title: string;
	place: string | null;
	starts_at: number;
	ends_at: number;
	over: boolean;
	member_count: number;
	game_count: number;
}

/** `GET /api/me/organised`. */
export interface Organised {
	events: OrganisedEvent[];
	matches: OrganisedMatch[];
	boards: OrganisedBoard[];
}
