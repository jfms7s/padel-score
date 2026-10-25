import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	addTeam,
	advanceByCourt,
	assignMatch,
	courtStatusText,
	courtStreamUrl,
	eventStreamUrl,
	eventViewStreamUrl,
	getCourt,
	listEvents,
	matchText,
	parseTypedScore,
	placesFromOrder,
	postEventResult,
	teamText,
	withOvertimeWinner
} from './events';
import { ApiError, setUnauthorizedHandler } from './api';
import type { CourtOut, EventMatch, EventTeam, Format } from './types';

const ONE_SET: Format = { sets: { deuce: 'advantage', sets: 'one', deciding: 'full_set' } };
const BEST_OF_3: Format = { sets: { deuce: 'golden_point', sets: 'best_of_3', deciding: 'full_set' } };
const SUPER: Format = { sets: { deuce: 'golden_point', sets: 'best_of_3', deciding: 'super_tie_break' } };
const TIMED: Format = { timed: { deuce: 'golden_point', draws_allowed: false } };
const TIMED_DRAWS: Format = { timed: { deuce: 'golden_point', draws_allowed: true } };

const set = (a: number, b: number, tiebreak: [number, number] | null = null) => ({ a, b, tiebreak });

describe('parseTypedScore', () => {
	it('parse_one_set', () => {
		expect(parseTypedScore('6-4', ONE_SET)).toEqual({ ok: { sets: [set(6, 4)] } });
		expect(parseTypedScore('  2-6 ', ONE_SET)).toEqual({ ok: { sets: [set(2, 6)] } });
	});

	it('parse_three_sets_with_super_tiebreak', () => {
		expect(parseTypedScore('6-4 3-6 10-7', SUPER)).toEqual({
			ok: { sets: [set(6, 4), set(3, 6), set(1, 0, [10, 7])] }
		});
		expect(parseTypedScore('4-6 6-3 8-10', SUPER)).toEqual({
			ok: { sets: [set(4, 6), set(6, 3), set(0, 1, [8, 10])] }
		});
		// Not one set all: no deciding super tie-break, the set stays as typed for the engine to judge.
		expect(parseTypedScore('6-4 6-3 10-7', SUPER)).toEqual({
			ok: { sets: [set(6, 4), set(6, 3), set(10, 7)] }
		});
		// A full deciding set is a set like any other.
		expect(parseTypedScore('6-4 3-6 7-5', BEST_OF_3)).toEqual({
			ok: { sets: [set(6, 4), set(3, 6), set(7, 5)] }
		});
	});

	it('parse_tiebreak_loser_points', () => {
		expect(parseTypedScore('7-6(5)', ONE_SET)).toEqual({ ok: { sets: [set(7, 6, [7, 5])] } });
		expect(parseTypedScore('6-7(3)', ONE_SET)).toEqual({ ok: { sets: [set(6, 7, [3, 7])] } });
		// Past 5, the winner needs two clear: max(7, loser + 2).
		expect(parseTypedScore('7-6(10)', ONE_SET)).toEqual({ ok: { sets: [set(7, 6, [12, 10])] } });
		expect(parseTypedScore('6-4 6-7 (8) 10-6', SUPER)).toEqual({
			ok: { sets: [set(6, 4), set(6, 7, [8, 10]), set(1, 0, [10, 6])] }
		});
	});

	it('parse_needs_tiebreak_for_7_6', () => {
		expect(parseTypedScore('7-6', ONE_SET)).toEqual({ error: 'needs_tiebreak' });
		expect(parseTypedScore('6-7', ONE_SET)).toEqual({ error: 'needs_tiebreak' });
		expect(parseTypedScore('6-4 7-6', BEST_OF_3)).toEqual({ error: 'needs_tiebreak' });
	});

	it('parse_dash_variants', () => {
		const want = { ok: { sets: [set(6, 4), set(3, 6), set(1, 0, [10, 7])] } };
		expect(parseTypedScore('6–4 3–6 10–7', SUPER)).toEqual(want);
		expect(parseTypedScore('6:4 3:6 10:7', SUPER)).toEqual(want);
		expect(parseTypedScore('6-4,3-6,10-7', SUPER)).toEqual(want);
		expect(parseTypedScore('6 - 4, 3 - 6, 10 - 7', SUPER)).toEqual(want);
	});

	it('parse_timed', () => {
		expect(parseTypedScore('5-3', TIMED)).toEqual({ ok: { timed: { a: 5, b: 3, overtime_winner: null } } });
		expect(parseTypedScore('2:7', TIMED_DRAWS)).toEqual({ ok: { timed: { a: 2, b: 7, overtime_winner: null } } });
		expect(parseTypedScore('5-3 6-2', TIMED)).toEqual({ error: 'syntax' });
		expect(parseTypedScore('7-6(5)', TIMED)).toEqual({ error: 'syntax' });
	});

	it('parse_timed_level_needs_overtime_winner', () => {
		const out = parseTypedScore('5-5', TIMED);
		expect(out).toMatchObject({ error: 'needs_overtime_winner' });
		if (!('error' in out) || out.error !== 'needs_overtime_winner') throw new Error('expected a winner prompt');
		expect(withOvertimeWinner(out.score, 'b')).toEqual({ timed: { a: 5, b: 5, overtime_winner: 'b' } });
		// A sets score has no overtime: unchanged.
		const sets = { sets: [set(6, 4)] };
		expect(withOvertimeWinner(sets, 'a')).toEqual(sets);
	});

	it('parse_timed_level_draws_ok', () => {
		expect(parseTypedScore('4-4', TIMED_DRAWS)).toEqual({ ok: { timed: { a: 4, b: 4, overtime_winner: null } } });
	});

	it('parse_garbage_is_syntax', () => {
		for (const text of ['abc', '6-', '-4', '6-4-3', '6-4 x', '7-6(5', '7-6()', '6/4', '6-4)', '1000-0', '6-4 (5)'])
			expect(parseTypedScore(text, ONE_SET), text).toEqual({ error: 'syntax' });
		expect(parseTypedScore('', ONE_SET)).toEqual({ error: 'empty' });
		expect(parseTypedScore(' ,  ', ONE_SET)).toEqual({ error: 'empty' });
	});
});

describe('places', () => {
	it('places_from_order_shares_places', () => {
		expect(placesFromOrder([['a'], ['b'], ['c', 'd']])).toEqual([
			{ team_id: 'a', place: 1, place_to: 1 },
			{ team_id: 'b', place: 2, place_to: 2 },
			{ team_id: 'c', place: 3, place_to: 4 },
			{ team_id: 'd', place: 3, place_to: 4 }
		]);
		expect(placesFromOrder([['a', 'b'], [], ['c']])).toEqual([
			{ team_id: 'a', place: 1, place_to: 2 },
			{ team_id: 'b', place: 1, place_to: 2 },
			{ team_id: 'c', place: 3, place_to: 3 }
		]);
		expect(placesFromOrder([])).toEqual([]);
	});
});

const team = (id: string, first: string, second: string, custom: string | null = null): EventTeam => ({
	id,
	name: custom ?? `${first} / ${second}`,
	custom_name: custom,
	players: [
		{ id: `${id}-1`, name: first },
		{ id: `${id}-2`, name: second }
	]
});
const TEAMS = [team('t1', 'Ana', 'Marta'), team('t2', 'Rui', 'João'), team('t3', 'Eva', 'Nuno', 'The Lobs')];

const match = (id: string, a: string, b: string): EventMatch => ({
	id,
	label: 'Match 1',
	ordinal: 1,
	team_a: a,
	team_b: b,
	court_id: 'c1',
	state: 'waiting',
	locked: false,
	winner: null,
	result: null,
	live: null,
	scorer_token: 'sc',
	spectator_token: 'sp'
});

const court = (phones: number, current: string | null): CourtOut => ({
	id: 'c1',
	name: 'Court 1',
	position: 1,
	scorer_token: 'cs',
	spectator_token: 'cv',
	phones,
	current,
	queue: []
});

describe('text', () => {
	it('team and match text', () => {
		expect(teamText(TEAMS[0]!)).toBe('Ana / Marta');
		expect(teamText(TEAMS[2]!)).toBe('The Lobs');
		expect(matchText(match('m1', 't1', 't2'), TEAMS)).toBe('Ana / Marta v Rui / João');
	});

	it('court_status_text_variants', () => {
		const matches = [match('m1', 't1', 't2')];
		expect(courtStatusText(court(1, 'm1'), matches, TEAMS)).toBe('Phone connected, Ana / Marta v Rui / João');
		expect(courtStatusText(court(2, 'm1'), matches, TEAMS)).toBe('Phone connected, Ana / Marta v Rui / João');
		expect(courtStatusText(court(0, 'm1'), matches, TEAMS)).toBe('No phone yet');
		expect(courtStatusText(court(0, null), matches, TEAMS)).toBe('No phone yet');
		expect(courtStatusText(court(1, null), matches, TEAMS)).toBe('Waiting for a match');
		// A current match not in the list (a frame from before it arrived): still waiting.
		expect(courtStatusText(court(1, 'gone'), matches, TEAMS)).toBe('Waiting for a match');
	});
});

describe('requests', () => {
	afterEach(() => vi.unstubAllGlobals());

	function stubFetch(status: number, body: unknown) {
		const fetch = vi.fn(async () => new Response(status === 204 ? null : JSON.stringify(body), { status }));
		vi.stubGlobal('fetch', fetch);
		return fetch;
	}
	const sent = (fetch: ReturnType<typeof stubFetch>) => {
		const [path, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
		return { path, method: init.method, body: init.body ? JSON.parse(init.body as string) : undefined };
	};

	it('stream urls', () => {
		expect(eventStreamUrl('e/1')).toBe('/api/events/e%2F1/stream');
		expect(courtStreamUrl('tok')).toBe('/api/court/tok/stream');
		expect(eventViewStreamUrl('tok')).toBe('/api/event-view/tok/stream');
	});

	it('coordinator calls use the event paths and redirect on 401', async () => {
		let f = stubFetch(200, { items: [] });
		await expect(listEvents()).resolves.toEqual([]);
		expect(sent(f)).toMatchObject({ path: '/api/events', method: 'GET' });

		f = stubFetch(201, team('t9', 'Ana', 'Rui'));
		await addTeam('e1', { players: ['p1', 'p2'] });
		expect(sent(f)).toEqual({ path: '/api/events/e1/teams', method: 'POST', body: { players: ['p1', 'p2'] } });

		f = stubFetch(200, match('m1', 't1', 't2'));
		await assignMatch('e1', 'm1', null);
		expect(sent(f)).toEqual({ path: '/api/events/e1/matches/m1', method: 'PATCH', body: { court_id: null } });

		let redirected = 0;
		const undo = setUnauthorizedHandler(() => redirected++);
		stubFetch(401, { error: 'unauthorized' });
		await expect(listEvents()).rejects.toBeInstanceOf(ApiError);
		undo();
		expect(redirected).toBe(1);
	});

	it('a refused result carries the engine reason', async () => {
		stubFetch(422, { error: 'invalid_score', reason: { impossible_set: { index: 0 } } });
		const err = await postEventResult('e1', 'm1', { client_id: 'c', score: { sets: [set(6, 5)] } }).catch((e: unknown) => e);
		expect(err).toBeInstanceOf(ApiError);
		expect(err).toMatchObject({ status: 422, code: 'invalid_score', reason: { impossible_set: { index: 0 } } });
	});

	it('court calls need no account', async () => {
		let redirected = 0;
		const undo = setUnauthorizedHandler(() => redirected++);
		let f = stubFetch(200, { court: {} });
		await getCourt('ct');
		expect(sent(f)).toMatchObject({ path: '/api/court/ct' });
		f = stubFetch(403, { error: 'forbidden' });
		await expect(advanceByCourt('ct')).rejects.toMatchObject({ status: 403, code: 'forbidden' });
		expect(sent(f)).toMatchObject({ path: '/api/court/ct/advance', method: 'POST' });
		undo();
		expect(redirected).toBe(0);
	});
});
