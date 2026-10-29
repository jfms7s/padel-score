import { describe, expect, it } from 'vitest';
import { longDate } from './dates';
import { STALE_SECS, agendaNote, agendaTitle, itemEnd, liveNow, offlineRows, shownHours, weekPips } from './home';
import type { RecentMatch } from './setup';
import type { CalendarItem, MatchView, MyMatch, Score } from './types';

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

	it('says a match with its place and open places', () => {
		expect(agendaNote(match(1, { place: 'Court 2', players: ['Pedro', 'Inês', null, null] }))).toBe('Court 2 · 2 open');
		const full = match(1, { place: 'Court 2', players: ['A', 'B', 'C', 'D'] });
		expect(agendaNote(full)).toBe('Court 2');
		expect(agendaNote(match(1, { players: ['A', 'B', 'C', 'D'] }))).toBe('');
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
});
