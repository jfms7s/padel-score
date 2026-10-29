import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	addTeam,
	advanceByCourt,
	assignMatch,
	courtLinks,
	courtStatusText,
	courtStreamUrl,
	eventLink,
	eventMessage,
	eventStreamUrl,
	eventViewStreamUrl,
	getCourt,
	groupsFromTies,
	invalidScoreKey,
	invalidScoreText,
	isEventDetail,
	listEvents,
	liveScoreText,
	matchText,
	matchViewPath,
	moveItem,
	parsedErrorText,
	parseTypedScore,
	placesFromOrder,
	postEventResult,
	resultRefusalText,
	rulesText,
	teamText,
	unfinishedCount,
	withOvertimeWinner
} from './events';
import { ApiError, setUnauthorizedHandler } from './api';
import { strings } from './strings';
import type { CourtOut, EventSummary, EventMatch, EventTeam, Format, InvalidScore, Score } from './types';

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
		const err = await postEventResult('e1', 'm1', { client_id: 'c', score: { sets: [set(6, 5)] } }).catch(
			(e: unknown) => e
		);
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

describe('coordinator screens', () => {
	it('typed_sheet_reason_text_covers_every_invalid_score', () => {
		// Every InvalidScore variant the engine can give (keep in step with types.ts).
		const all: InvalidScore[] = [
			{ invalid_rules: 'super_tie_break_needs_multiple_sets' },
			{ invalid_rules: 'invalid_player_index' },
			'wrong_format',
			{ impossible_set: { index: 1 } },
			{ missing_tie_break: { index: 0 } },
			'match_not_finished',
			'extra_sets',
			'draw_not_allowed',
			'overtime_winner_when_not_level'
		];
		const table: Record<string, unknown> = strings.events.invalidScore;
		const keys = new Set(all.map(invalidScoreKey));
		// One strings entry per variant, and no stale ones.
		expect([...keys].sort()).toEqual(Object.keys(table).sort());
		const texts = all.map(invalidScoreText);
		for (const text of texts) expect(text.trim().length).toBeGreaterThan(0);
		expect(new Set(texts).size).toBe(texts.length);
		// Set numbers are counted from one.
		expect(invalidScoreText({ impossible_set: { index: 1 } })).toContain('2');
		expect(invalidScoreText({ missing_tie_break: { index: 0 } })).toContain('1');
		// The parser's own reasons have words too.
		for (const e of ['empty', 'syntax', 'needs_tiebreak', 'needs_overtime_winner'] as const)
			expect(parsedErrorText(e).length).toBeGreaterThan(0);
	});

	it('qr_links_use_origin_and_tokens', () => {
		const links = courtLinks('https://padel.example', court(0, null));
		expect(links).toEqual({ scoring: 'https://padel.example/c/cs', viewing: 'https://padel.example/cv/cv' });
		expect(eventLink('https://padel.example', 'ev')).toBe('https://padel.example/e/ev');
		expect(matchViewPath('sp')).toBe('/v/sp');
	});

	it('rules_text', () => {
		expect(rulesText(ONE_SET)).toBe('1 set, advantage');
		expect(rulesText(SUPER)).toBe('Best of 3, super tie-break, golden point');
		expect(rulesText(BEST_OF_3)).toBe('Best of 3, full deciding set, golden point');
		expect(rulesText(TIMED)).toBe('Timed, golden point, no draws');
		expect(rulesText(TIMED_DRAWS)).toBe('Timed, golden point, draws allowed');
	});

	it('live_score_text', () => {
		const base: Score = {
			sets: [set(6, 4), set(2, 1)],
			sets_won: [1, 0],
			games: [2, 1],
			regulation_games: [2, 1],
			current: { game: { a: 'thirty', b: 'fifteen', deuces: 0, deciding_point: false } },
			server: { team: 'a', index: 0 },
			side: 'right',
			change_ends: false,
			overtime: false,
			time_up: false,
			status: 'in_progress'
		};
		expect(liveScoreText(base)).toBe('6-4 2-1, 30-15');
		expect(
			liveScoreText({ ...base, sets: [set(6, 6)], current: { tie_break: { a: 3, b: 2, super_tiebreak: false } } })
		).toBe('6-6, 3-2');
		expect(
			liveScoreText({
				...base,
				sets: [],
				games: [4, 3],
				current: { game: { a: 'love', b: 'forty', deuces: 0, deciding_point: false } }
			})
		).toBe('4-3, 0-40');
		expect(liveScoreText({ ...base, sets: [], games: [4, 3], current: 'none' })).toBe('4-3');
	});

	it('event_message', () => {
		expect(eventMessage(new ApiError(409, 'too_few_teams', 'x'))).toBe(strings.events.errors.too_few_teams);
		expect(eventMessage(new ApiError(409, 'no_courts', 'x'))).toBe(strings.events.errors.no_courts);
		expect(eventMessage(new ApiError(409, 'odd_teams', 'x'))).toBe('A non-stop needs an even number of teams.');
		expect(eventMessage(new ApiError(409, 'schedule_too_big', 'x'))).toBe(
			'That’s too many matches: use fewer teams or legs.'
		);
		expect(eventMessage(new ApiError(422, 'validation', 'x', false, { field: 'name' }))).toBe(
			strings.events.fields.name
		);
		expect(eventMessage(new ApiError(0, 'network', 'x'))).toBe(strings.networkError);
		expect(eventMessage(new Error('boom'))).toBe(strings.networkError);
	});

	it('result_refusal_text', () => {
		const reason = { impossible_set: { index: 0 } } as const;
		expect(resultRefusalText(new ApiError(422, 'invalid_score', 'x', false, { reason }))).toBe(
			invalidScoreText(reason)
		);
		expect(resultRefusalText(new ApiError(409, 'conflict', 'x'))).toBe(strings.events.sheet.conflict);
		expect(resultRefusalText(new ApiError(409, 'locked', 'x'))).toBe(strings.events.sheet.locked);
		expect(resultRefusalText(new ApiError(409, 'no_courts', 'x'))).toBe(
			eventMessage(new ApiError(409, 'no_courts', 'x'))
		);
		expect(resultRefusalText(new Error('boom'))).toBe(strings.networkError);
	});

	it('finish_warns_about_unfinished_matches', () => {
		const done = { ...match('m3', 't1', 't3'), state: 'finished' as const };
		const live = { ...match('m2', 't2', 't3'), state: 'live' as const };
		expect(unfinishedCount([match('m1', 't1', 't2'), live, done])).toBe(2);
		expect(unfinishedCount([done])).toBe(0);
		expect(unfinishedCount([])).toBe(0);
		expect(strings.events.unfinishedWarning(2)).toBe(
			'2 matches aren’t finished. Their court phones will stop working.'
		);
		expect(strings.events.unfinishedWarning(1)).toBe('1 match isn’t finished. Its court phone will stop working.');
	});

	it('places_editor_helpers', () => {
		expect(moveItem(['a', 'b', 'c'], 0, 2)).toEqual(['b', 'c', 'a']);
		expect(moveItem(['a', 'b', 'c'], 2, 1)).toEqual(['a', 'c', 'b']);
		expect(moveItem(['a', 'b'], 0, 5)).toEqual(['a', 'b']);
		expect(groupsFromTies(['a', 'b', 'c', 'd'], [false, false, true])).toEqual([['a'], ['b'], ['c', 'd']]);
		expect(groupsFromTies(['a', 'b', 'c'], [true, true])).toEqual([['a', 'b', 'c']]);
		expect(groupsFromTies(['a', 'b'], [])).toEqual([['a'], ['b']]);
		expect(groupsFromTies([], [])).toEqual([]);
	});

	it('is_event_detail', () => {
		const detail = {
			event: { id: 'e1', name: 'Sunday cup', status: 'setup' },
			spectator_token: 'ev',
			teams: [],
			courts: [],
			matches: [],
			places: [],
			places_computed: false,
			kind_data: null
		};
		expect(isEventDetail(detail)).toBe(true);
		expect(isEventDetail({ ...detail, courts: null })).toBe(false);
		expect(isEventDetail(null)).toBe(false);
		expect(isEventDetail({ ...detail, event: 'x' })).toBe(false);
	});
});
