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
}

/** Internally tagged on `kind`. */
export type Command =
	| { kind: 'point'; team: Team }
	| { kind: 'time_up' }
	| { kind: 'undo'; target_seq: number };

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
}

export interface CreatedMatch {
	id: string;
	scorer_token: string;
	spectator_token: string;
}

/** Every non-2xx body the API sends has an `error` code. */
export type ApiErrorBody =
	| { error: 'conflict'; view: MatchView }
	| { error: 'rejected'; kind: ErrorKind; index: number }
	| { error: 'invalid'; message: string }
	| { error: 'invalid_match'; message: string }
	| {
			error:
				| 'not_found'
				| 'bad_request'
				| 'method_not_allowed'
				| 'payload_too_large'
				| 'internal';
	  }
	| { error: 'validation'; field: string }
	| { error: 'too_many_requests'; retry_after_secs: number }
	| { error: AccountErrorCode };

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
	event_kind: 'tournament' | 'non_stop';
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

// ---- linking, claims and privacy (crates/server/src/linking_api.rs) ----------------------

/** `Player.link`: who an entry is linked to. */
export interface PlayerLink {
	display_name: string;
	linked_at: number;
	is_me: boolean;
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

/** What a claim points at. */
export type ClaimTarget = { kind: 'match_slot'; spectator_token: string; team: Team; position: number };

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

/** One match of a combined history; the redacted form has no names, coordinator or link. */
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
	  }
	| {
			match_id: string;
			finished_at: number;
			outcome: Outcome;
			side: Team;
			score: FinalScore;
			redacted: true;
	  };

/** `GET /api/me/results`, `/api/players/{id}/linked-history` and `/api/profiles/{token}`. */
export interface LinkedHistory extends Page<LinkedEntry> {
	display_name: string;
	record: PlayerRecord;
	placings: Placing[];
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
