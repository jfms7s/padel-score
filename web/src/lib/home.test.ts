import { describe, expect, it } from 'vitest';
import { longDate } from './dates';
import {
	STALE_SECS,
	agendaNote,
	agendaTitle,
	itemEnd,
	itemKey,
	isCancelled,
	laneLayout,
	liveNow,
	openEnd,
	offlineRows,
	shownHours,
	weekPips
} from './home';
import type { RecentMatch } from './setup';
import type { CalendarItem, MatchView, MyMatch, OpenItem, Score } from './types';

type Game = Extract<CalendarItem, { kind: 'game' }>;
type Event = Extract<CalendarItem, { kind: 'event' }>;
type Match = Extract<CalendarItem, { kind: 'match' }>;

const game = (start: number, over: Partial<Game> = {}): CalendarItem => ({
	kind: 'game',
	board_token: 'bt',
	board_title: 'Padel this week',
	game_id: 'g1',
	start,
	end: start + 5400,
	players: ['Ana', 'Rui', 'João', 'Marta'],
	state: 'full',
	board_place: null,
	me: 0,
	...over
});
const event = (starts_at: number, over: Partial<Event> = {}): CalendarItem => ({
	kind: 'event',
	event_id: null,
	join_token: 'jt',
	name: 'Sunday cup',
	event_kind: 'tournament',
	starts_at,
	place: null,
	role: 'player',
	team: ['Ana', 'Rui'],
	place_in_line: null,
	status: 'setup',
	teams: 0,
	max_teams: 8,
	...over
});
const match = (starts_at: number, over: Partial<Match> = {}): CalendarItem => ({
	kind: 'match',
	join_token: 'mt',
	starts_at,
	place: null,
	role: 'player',
	players: ['Ana', null, null, null],
	state: 'open',
	owner_name: 'Pedro',
	...over
});

const openMatch = (starts_at: number): OpenItem => ({
	kind: 'match',
	join_token: 'om',
	owner_name: 'Pedro',
	starts_at,
	place: null,
	open_places: 2
});
const openEvent = (starts_at: number): OpenItem => ({
	kind: 'event',
	join_token: 'oe',
	name: 'Cup',
	event_kind: 'tournament',
	starts_at,
	place: null,
	teams: 1,
	max_teams: 8
});

const SCORE = { sets: [{ a: 6, b: 4 }], games: [2, 1], current: 'none' } as unknown as Score;

const mine = (over: Partial<MyMatch> = {}): MyMatch => ({
	id: 'm1',
	label: 'Ana & Rui vs Marta & Joana',
	created_at: 0,
	last_activity_at: 0,
	view: { score: SCORE } as MatchView,
	scorer_token: 'st',
	spectator_token: 'vt',
	...over
});

const NOW = Date.UTC(2026, 9, 28, 12) / 1000;
// The week of Monday 26 Oct 2026.
const THU = (h: number, m = 0) => Date.UTC(2026, 9, 29, h, m) / 1000;
const SAT = (h: number) => Date.UTC(2026, 9, 31, h) / 1000;

describe('liveNow', () => {
	it('drops entries without a scorer token and keeps server order', () => {
		const a = mine({ id: 'a', last_activity_at: NOW });
		const b = mine({ id: 'b', scorer_token: undefined, last_activity_at: NOW });
		const c = mine({ id: 'c', scorer_token: 'sc', last_activity_at: NOW });
		const { fresh, older } = liveNow([a, b, c], NOW, 'UTC');
		expect(fresh.map((r) => r.scorerToken)).toEqual(['st', 'sc']);
		expect(older).toEqual([]);
	});

	it('splits at exactly a day of silence', () => {
		const edge = mine({ id: 'edge', scorer_token: 'e', last_activity_at: NOW - STALE_SECS });
		const stale = mine({ id: 'old', scorer_token: 'o', last_activity_at: NOW - STALE_SECS - 1 });
		const { fresh, older } = liveNow([stale, edge], NOW, 'UTC');
		expect(fresh.map((r) => r.scorerToken)).toEqual(['e']);
		expect(older.map((r) => r.scorerToken)).toEqual(['o']);
	});

	it('builds the row from the match', () => {
		const [row] = liveNow([mine({ last_activity_at: NOW })], NOW, 'UTC').fresh;
		expect(row).toEqual({
			label: 'Ana & Rui vs Marta & Joana',
			href: '/s/st#v=vt',
			scorerToken: 'st',
			score: '6-4',
			when: longDate(NOW, 'UTC'),
			deletable: true
		});
	});

	it('links without a spectator token and keeps event matches from being deleted', () => {
		const [row] = liveNow(
			[mine({ spectator_token: undefined, context: 'Court 1 · Sunday cup', last_activity_at: NOW })],
			NOW,
			'UTC'
		).fresh;
		expect(row?.href).toBe('/s/st');
		expect(row?.deletable).toBe(false);
	});

	it('is empty for nothing', () => {
		expect(liveNow([], NOW, 'UTC')).toEqual({ fresh: [], older: [] });
	});
});

describe('offlineRows', () => {
	it('maps recent matches, whose created_at is in milliseconds', () => {
		const recent: RecentMatch[] = [
			{ scorer_token: 'a', spectator_token: 'v', label: 'One', created_at: NOW * 1000 },
			{ scorer_token: 'b', label: 'Two', created_at: (NOW + 86_400) * 1000 }
		];
		expect(offlineRows(recent, 'UTC')).toEqual([
			{ label: 'One', href: '/s/a#v=v', scorerToken: 'a', score: null, when: 'Wed 28 Oct', deletable: true },
			{ label: 'Two', href: '/s/b', scorerToken: 'b', score: null, when: 'Thu 29 Oct', deletable: true }
		]);
	});
});

describe('weekPips', () => {
	it('lists seven days from Monday with the kinds in item order', () => {
		const pips = weekPips([game(THU(19)), event(THU(10))], '2026-10-26', 'UTC');
		expect(pips.map((p) => p.date)).toEqual([
			'2026-10-26',
			'2026-10-27',
			'2026-10-28',
			'2026-10-29',
			'2026-10-30',
			'2026-10-31',
			'2026-11-01'
		]);
		expect(pips[3]?.kinds).toEqual(['game', 'event']);
		expect(pips.filter((p) => p.kinds.length === 0)).toHaveLength(6);
	});

	it('gives one entry per item, so two games give two', () => {
		const pips = weekPips([game(THU(9)), game(THU(19), { game_id: 'g2' }), match(THU(20))], '2026-10-26', 'UTC');
		expect(pips[3]?.kinds).toEqual(['game', 'game', 'match']);
	});

	it('ignores items outside the week and reads days in the zone', () => {
		const before = game(Date.UTC(2026, 9, 25, 12) / 1000);
		const after = game(Date.UTC(2026, 10, 2, 12) / 1000);
		expect(weekPips([before, after], '2026-10-26', 'UTC').every((p) => p.kinds.length === 0)).toBe(true);
		// 23:30 UTC on Sunday 1 Nov is Monday 2 Nov in Tokyo, outside the week.
		const late = event(Date.UTC(2026, 10, 1, 23, 30) / 1000);
		expect(weekPips([late], '2026-10-26', 'Asia/Tokyo').every((p) => p.kinds.length === 0)).toBe(true);
		expect(weekPips([late], '2026-10-26', 'UTC')[6]?.kinds).toEqual(['event']);
	});
});

describe('agendaTitle', () => {
	it('names the other players and "you" for a game', () => {
		expect(agendaTitle(game(1))).toBe('Rui, João, Marta & you');
		expect(agendaTitle(game(1, { me: 2 }))).toBe('Ana, Rui, Marta & you');
	});

	it('names an event by its name', () => {
		expect(agendaTitle(event(1))).toBe('Sunday cup');
	});

	it('names a match by its owner, or by its place for the organiser', () => {
		expect(agendaTitle(match(1))).toBe('Pedro’s match');
		expect(agendaTitle(match(1, { role: 'organiser' }))).toBe('Match');
		expect(agendaTitle(match(1, { role: 'organiser', place: 'Court 2' }))).toBe('Match · Court 2');
		expect(agendaTitle(match(1, { role: 'line', place: 'Court 2' }))).toBe('Pedro’s match');
	});
});

describe('agendaNote', () => {
	it('says a game with its place and board title, and what it needs', () => {
		const full = game(1, { board_place: 'Clube Oeiras' });
		expect(agendaNote(full)).toBe('Game · Clube Oeiras · Padel this week');
		expect(agendaNote(game(1, { board_place: 'Clube Oeiras', state: 'short' }))).toBe(
			'Game · Clube Oeiras · Padel this week · needs a player'
		);
		expect(agendaNote(game(1))).toBe('Game · Padel this week');
	});

	it('says an event with its teams and the viewer in line', () => {
		const organiser = event(1, { place: 'Club', teams: 4, max_teams: 6, role: 'organiser' });
		expect(agendaNote(organiser)).toBe('Club · 4 of 6 teams');
		expect(agendaNote(event(1, { place: 'Club', teams: 4, max_teams: null, role: 'organiser' }))).toBe(
			'Club · 4 teams'
		);
		const line = agendaNote(event(1, { place: 'Club', teams: 4, max_teams: 6, role: 'line' }));
		expect(line.endsWith(' · in line')).toBe(true);
		expect(line).toBe('Club · 4 of 6 teams · in line');
	});

	it('leads a cancelled event with the cancelled word', () => {
		expect(agendaNote(event(1, { place: 'Club', teams: 4, max_teams: null, status: 'cancelled' }))).toBe(
			'Cancelled · Club · 4 teams'
		);
	});

	it('leads a cancelled match with the cancelled word and a started one with the started word', () => {
		const players = ['A', 'B', null, null];
		expect(agendaNote(match(1, { place: 'Court 2', players, state: 'cancelled' }))).toBe(
			'Cancelled · Court 2 · 2 open'
		);
		expect(agendaNote(match(1, { place: 'Court 2', players, state: 'started' }))).toBe('Started · Court 2 · 2 open');
	});

	it('says a match with its place and open places', () => {
		expect(agendaNote(match(1, { place: 'Court 2', players: ['Pedro', 'Inês', null, null] }))).toBe('Court 2 · 2 open');
		const full = match(1, { place: 'Court 2', players: ['A', 'B', 'C', 'D'] });
		expect(agendaNote(full)).toBe('Court 2');
		expect(agendaNote(match(1, { players: ['A', 'B', 'C', 'D'] }))).toBe('');
	});
});

describe('isCancelled', () => {
	it('is true for a cancelled event or match only', () => {
		expect(isCancelled(event(1, { status: 'cancelled' }))).toBe(true);
		expect(isCancelled(event(1))).toBe(false);
		expect(isCancelled(match(1, { state: 'cancelled' }))).toBe(true);
		expect(isCancelled(match(1, { state: 'started' }))).toBe(false);
		expect(isCancelled(game(1))).toBe(false);
	});
});

describe('itemKey', () => {
	it('keys a game by its id', () => expect(itemKey(game(1000, { game_id: 'g7' }))).toBe('game:g7'));
	it('keys an event by its id, else its join token', () => {
		expect(itemKey(event(1000, { event_id: 'e1' }))).toBe('event:e1');
		expect(itemKey(event(1000))).toBe('event:jt');
	});
	it('keys a match by its join token', () => expect(itemKey(match(1000))).toBe('match:mt'));
});

describe('openEnd', () => {
	it('draws an open match and an open event for their usual length', () => {
		expect(openEnd(openMatch(1000))).toBe(1000 + 5400);
		expect(openEnd(openEvent(1000))).toBe(1000 + 7200);
	});
});

describe('laneLayout', () => {
	it('gives a lone block the whole width', () => {
		expect(laneLayout([{ start: 0, end: 10 }])).toEqual([{ lane: 0, lanes: 1 }]);
	});
	it('splits overlapping blocks side by side, in order', () => {
		const out = laneLayout([
			{ start: 0, end: 10 },
			{ start: 5, end: 15 },
			{ start: 20, end: 30 }
		]);
		expect(out).toEqual([
			{ lane: 0, lanes: 2 },
			{ lane: 1, lanes: 2 },
			{ lane: 0, lanes: 1 }
		]);
	});
	it('reuses a lane that has ended', () => {
		const out = laneLayout([
			{ start: 0, end: 10 },
			{ start: 5, end: 8 },
			{ start: 9, end: 12 }
		]);
		expect(out.map((o) => o.lane)).toEqual([0, 1, 1]);
	});
});

describe('itemEnd', () => {
	it('uses the game end and guesses the others', () => {
		expect(itemEnd(game(1000, { end: 4000 }))).toBe(4000);
		expect(itemEnd(match(1000))).toBe(1000 + 5400);
		expect(itemEnd(event(1000))).toBe(1000 + 7200);
	});
});

describe('shownHours', () => {
	it('covers every hour an item touches, ascending and unique', () => {
		const items = [event(SAT(10)), game(THU(19), { end: THU(20, 30) })];
		expect(shownHours(items, '2026-10-26', 'UTC')).toEqual([10, 11, 19, 20]);
	});

	it('shows a half-past start from its own hour', () => {
		expect(shownHours([game(THU(19, 30), { end: THU(20, 30) })], '2026-10-26', 'UTC')).toEqual([19, 20]);
	});

	it('falls back to the evening for an empty week', () => {
		expect(shownHours([], '2026-10-26', 'UTC')).toEqual([18, 19, 20, 21]);
		expect(shownHours([game(Date.UTC(2026, 10, 3, 9) / 1000)], '2026-10-26', 'UTC')).toEqual([18, 19, 20, 21]);
	});

	it('reads hours in the viewer zone', () => {
		// 10:00 UTC is 19:00 in Tokyo (UTC+9), and an event runs two hours.
		expect(shownHours([event(THU(10))], '2026-10-26', 'Asia/Tokyo')).toEqual([19, 20]);
	});

	it('counts open items too', () => {
		expect(shownHours([], '2026-10-26', 'UTC', [openMatch(SAT(9))])).toEqual([9, 10]);
		expect(shownHours([game(THU(19))], '2026-10-26', 'UTC', [openEvent(SAT(9))])).toEqual([9, 10, 19, 20]);
	});
});
