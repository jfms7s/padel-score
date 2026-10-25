// Runtime shape checks for the wire types in ./types. They are strict: every field must be
// present with the right type and no unknown field is allowed, so a change in serde's output
// (a renamed field, a new variant) fails here instead of slipping through a cast.

import type { Event, Format, MatchView, Score } from './types';

type Check = (v: unknown) => boolean;

const isObj = (v: unknown): v is Record<string, unknown> =>
	typeof v === 'object' && v !== null && !Array.isArray(v);

/** An object with exactly these keys, each passing its check. */
const exact =
	(shape: Record<string, Check>): Check =>
	(v) => {
		if (!isObj(v)) return false;
		const keys = Object.keys(v);
		return (
			keys.length === Object.keys(shape).length &&
			keys.every((k) => k in shape && shape[k]!(v[k]))
		);
	};

/** A serde externally tagged variant: an object with one key. */
const tagged =
	(variants: Record<string, Check>): Check =>
	(v) => {
		if (!isObj(v)) return false;
		const keys = Object.keys(v);
		return keys.length === 1 && keys[0]! in variants && variants[keys[0]!]!(v[keys[0]!]);
	};

const oneOf =
	(...values: unknown[]): Check =>
	(v) =>
		values.includes(v);
const either =
	(...checks: Check[]): Check =>
	(v) =>
		checks.some((c) => c(v));
const int: Check = (v) => Number.isInteger(v) && (v as number) >= 0;
const bool: Check = (v) => typeof v === 'boolean';
const str: Check = (v) => typeof v === 'string';
const tuple =
	(...items: Check[]): Check =>
	(v) =>
		Array.isArray(v) && v.length === items.length && items.every((c, i) => c(v[i]));
const array =
	(item: Check): Check =>
	(v) =>
		Array.isArray(v) && v.every(item);
const nullable =
	(c: Check): Check =>
	(v) =>
		v === null || c(v);

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

const matchView = exact({
	id: str,
	format,
	start,
	teams: exact({ a: tuple(str, str), b: tuple(str, str) }),
	revision: int,
	head_seq: int,
	events: array(exact({ seq: int, event })),
	score
});

export const isFormat = (v: unknown): v is Format => format(v);
export const isEvent = (v: unknown): v is Event => event(v);
export const isScore = (v: unknown): v is Score => score(v);
export const isMatchView = (v: unknown): v is MatchView => matchView(v);
