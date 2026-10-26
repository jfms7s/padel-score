// Runtime shape checks for the wire types in ./types.
//
// Two modes. Strict (the fixture tests): every field present with the right type and no unknown
// field allowed, so a change in serde's output (a renamed field, a new field) fails a test instead
// of slipping through a cast. Lenient (the default, for what arrives at runtime: SSE frames, API
// answers, stored views and queues): the required fields with their types, extra keys ignored, so
// a server that adds a MatchView field doesn't freeze the board. Unknown enum values and variants
// fail in both modes: those are values the app can't show or score.

import type {
	AdminUser,
	CreatedInvite,
	Device,
	DeviceLookup,
	Event,
	Format,
	Invite,
	InviteLookup,
	MatchView,
	Me,
	MyMatch,
	Page,
	Player,
	PlayerPage,
	HistoryEntry,
	ResetLink,
	Score,
	SetupStatus
} from './types';

export interface GuardOptions {
	/** Refuse unknown keys too (the fixture tests). Default false. */
	strict?: boolean;
}

export type Check = (v: unknown) => boolean;

export const isObj = (v: unknown): v is Record<string, unknown> =>
	typeof v === 'object' && v !== null && !Array.isArray(v);

export type Shape = Record<string, Check>;

/** The `optional` keys that are present pass their checks (serde's `skip_serializing_if`). */
const optionalOk = (v: Record<string, unknown>, optional: Shape) =>
	Object.entries(optional).every(([k, c]) => !Object.hasOwn(v, k) || c(v[k]));

/** An object with exactly these keys (plus any of the `optional` ones), each passing its check. */
const exactly =
	(shape: Shape, optional: Shape = {}): Check =>
	(v) => {
		if (!isObj(v)) return false;
		return (
			Object.keys(v).every((k) => Object.hasOwn(shape, k) || Object.hasOwn(optional, k)) &&
			Object.entries(shape).every(([k, c]) => Object.hasOwn(v, k) && c(v[k])) &&
			optionalOk(v, optional)
		);
	};

/** An object with at least these keys, each passing its check; others are ignored. */
export const atLeast =
	(shape: Shape, optional: Shape = {}): Check =>
	(v) =>
		isObj(v) && Object.entries(shape).every(([k, c]) => Object.hasOwn(v, k) && c(v[k])) && optionalOk(v, optional);

/** A serde externally tagged variant: an object with one key. */
const tagged =
	(variants: Record<string, Check>): Check =>
	(v) => {
		if (!isObj(v)) return false;
		const keys = Object.keys(v);
		return keys.length === 1 && Object.hasOwn(variants, keys[0]!) && variants[keys[0]!]!(v[keys[0]!]);
	};

export const oneOf =
	(...values: unknown[]): Check =>
	(v) =>
		values.includes(v);
export const either =
	(...checks: Check[]): Check =>
	(v) =>
		checks.some((c) => c(v));
export const int: Check = (v) => Number.isInteger(v) && (v as number) >= 0;
const bool: Check = (v) => typeof v === 'boolean';
export const str: Check = (v) => typeof v === 'string';
const tuple =
	(...items: Check[]): Check =>
	(v) =>
		Array.isArray(v) && v.length === items.length && items.every((c, i) => c(v[i]));
export const array =
	(item: Check): Check =>
	(v) =>
		Array.isArray(v) && v.every(item);
export const nullable =
	(c: Check): Check =>
	(v) =>
		v === null || c(v);

function build(exact: (shape: Shape, optional?: Shape) => Check) {
	const team = oneOf('a', 'b');
	const deuce = oneOf('advantage', 'golden_point', 'star_point');
	const call = oneOf('love', 'fifteen', 'thirty', 'forty', 'advantage');
	const player = exact({ team, index: oneOf(0, 1) });

	const format = tagged({
		sets: exact({
			deuce,
			sets: oneOf('one', 'best_of_3'),
			deciding: oneOf('full_set', 'super_tie_break')
		}),
		timed: exact({ deuce, draws_allowed: bool })
	});
	const start = exact({ first_server: player, other_first_server: oneOf(0, 1) });
	const event = either(oneOf('time_up'), tagged({ point: team }));

	const score = exact({
		sets: array(exact({ a: int, b: int, tiebreak: nullable(tuple(int, int)) })),
		sets_won: tuple(int, int),
		games: tuple(int, int),
		regulation_games: tuple(int, int),
		current: either(
			oneOf('none'),
			tagged({
				game: exact({ a: call, b: call, deuces: int, deciding_point: bool }),
				tie_break: exact({ a: int, b: int, super_tiebreak: bool })
			})
		),
		server: player,
		side: oneOf('right', 'left', 'receiver_chooses'),
		change_ends: bool,
		overtime: bool,
		time_up: bool,
		status: either(oneOf('in_progress'), tagged({ finished: exact({ winner: nullable(team) }) }))
	});

	const matchView = exact(
		{
			id: str,
			format,
			start,
			teams: exact({ a: tuple(str, str), b: tuple(str, str) }),
			revision: int,
			head_seq: int,
			events: array(exact({ seq: int, event })),
			score
		},
		{ locked: bool }
	);

	// Accounts. Times are Unix seconds.
	const me = exact({ id: str, email: str, display_name: str, is_admin: bool });
	const setupStatus = exact({ needs_setup: bool });
	const createdInvite = exact({ id: str, url: str, expires_at: int });
	const invite = exact(
		{ id: str, created_at: int, expires_at: int, status: oneOf('active', 'used', 'expired', 'revoked') },
		{ used_by_name: str }
	);
	const inviteLookup = exact({ invited_by_name: str, expires_at: int });
	const adminUser = exact(
		{ id: str, email: str, display_name: str, is_admin: bool, disabled: bool, created_at: int },
		{ invited_by_name: str }
	);
	const resetLink = exact({ url: str, expires_at: int });
	const deviceLookup = exact({ device_name: str, created_secs_ago: int, expires_in_secs: int });
	const device = exact({ id: str, name: str, prefix: str, created_at: int }, { last_used_at: nullable(int) });
	const myMatch = exact(
		{ id: str, label: str, created_at: int, last_activity_at: int, view: matchView },
		{ scorer_token: str, spectator_token: str, context: str }
	);

	// Players and history.
	const slot = nullable(str);
	const listedPlayer = exact({
		id: str,
		name: str,
		archived: bool,
		created_at: int,
		record: exact({ wins: int, losses: int, draws: int }),
		link: nullable(exact({ display_name: str, linked_at: int, is_me: bool })),
		pending_claims: int
	});
	const historyShape: Shape = {
		match_id: str,
		finished_at: int,
		teams: exact({ a: tuple(str, str), b: tuple(str, str) }),
		players: exact({ a: tuple(slot, slot), b: tuple(slot, slot) }),
		winner: nullable(team),
		score: exact({
			sets: array(exact({ a: int, b: int, tiebreak: nullable(tuple(int, int)) })),
			games: tuple(int, int),
			regulation_games: tuple(int, int),
			overtime: bool,
			timed: bool
		}),
		spectator_token: str
	};
	const eventKind = oneOf('tournament', 'non_stop');
	// Event matches only.
	const historyOptional: Shape = { event: exact({ id: str, name: str, kind: eventKind, label: str }) };
	const historyEntry = exact(historyShape, historyOptional);
	const playerHistoryEntry = exact(
		{
			...historyShape,
			side: team,
			outcome: oneOf('win', 'loss', 'draw')
		},
		historyOptional
	);
	const placing = exact({
		event_id: str,
		event_name: str,
		event_kind: eventKind,
		place: int,
		place_to: int,
		field_size: int,
		finished_at: int
	});
	const next = nullable(str);
	const playerList = exact({ items: array(listedPlayer) });
	const historyPage = exact({ items: array(historyEntry), next });
	const playerPage = exact({ player: listedPlayer, placings: array(placing), items: array(playerHistoryEntry), next });
	return {
		format,
		event,
		score,
		matchView,
		me,
		setupStatus,
		createdInvite,
		invite,
		inviteLookup,
		adminUser,
		resetLink,
		deviceLookup,
		device,
		myMatch,
		player: listedPlayer,
		playerList,
		historyEntry,
		historyPage,
		playerPage
	};
}

const strictChecks = build(exactly);
const lenientChecks = build(atLeast);
const pick = (o?: GuardOptions) => (o?.strict ? strictChecks : lenientChecks);

export const isFormat = (v: unknown, o?: GuardOptions): v is Format => pick(o).format(v);
export const isEvent = (v: unknown, o?: GuardOptions): v is Event => pick(o).event(v);
export const isScore = (v: unknown, o?: GuardOptions): v is Score => pick(o).score(v);
export const isMatchView = (v: unknown, o?: GuardOptions): v is MatchView => pick(o).matchView(v);

export const isMe = (v: unknown, o?: GuardOptions): v is Me => pick(o).me(v);
export const isSetupStatus = (v: unknown, o?: GuardOptions): v is SetupStatus => pick(o).setupStatus(v);
export const isCreatedInvite = (v: unknown, o?: GuardOptions): v is CreatedInvite => pick(o).createdInvite(v);
export const isInvite = (v: unknown, o?: GuardOptions): v is Invite => pick(o).invite(v);
export const isInviteLookup = (v: unknown, o?: GuardOptions): v is InviteLookup => pick(o).inviteLookup(v);
export const isAdminUser = (v: unknown, o?: GuardOptions): v is AdminUser => pick(o).adminUser(v);
export const isResetLink = (v: unknown, o?: GuardOptions): v is ResetLink => pick(o).resetLink(v);
export const isDeviceLookup = (v: unknown, o?: GuardOptions): v is DeviceLookup => pick(o).deviceLookup(v);
export const isDevice = (v: unknown, o?: GuardOptions): v is Device => pick(o).device(v);
export const isMyMatch = (v: unknown, o?: GuardOptions): v is MyMatch => pick(o).myMatch(v);
export const isPlayer = (v: unknown, o?: GuardOptions): v is Player => pick(o).player(v);
export const isPlayerList = (v: unknown, o?: GuardOptions): v is { items: Player[] } => pick(o).playerList(v);
export const isHistoryEntry = (v: unknown, o?: GuardOptions): v is HistoryEntry => pick(o).historyEntry(v);
export const isHistoryPage = (v: unknown, o?: GuardOptions): v is Page<HistoryEntry> => pick(o).historyPage(v);
export const isPlayerPage = (v: unknown, o?: GuardOptions): v is PlayerPage => pick(o).playerPage(v);
/** A list whose every item passes `item` (lenient). */
export const isListOf =
	<T>(item: (v: unknown) => v is T) =>
	(v: unknown): v is T[] =>
		Array.isArray(v) && v.every((x) => item(x));
