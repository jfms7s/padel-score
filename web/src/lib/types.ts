// TS mirrors of the JSON that serde writes for the engine (crates/scoring) and the server
// (crates/server). Every union here was settled from a real serialized sample; those samples
// live in ./fixtures and the tests load them through these types.

// ---- engine: rules and events ------------------------------------------------------------

export type Team = 'a' | 'b';

export interface Player {
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
	first_server: Player;
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
	server: Player;
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
	  };

// ---- WASM bridge -------------------------------------------------------------------------

/** What the engine's `replay` reports: an event it refused, or input it could not parse. */
export type EngineError = { kind: ErrorKind; index: number } | { kind: 'bad_json'; message: string };

export type ReplayResult = { ok: Score } | { err: EngineError };
