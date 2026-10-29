import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	PUSH_PROMPT_KEY,
	byDay,
	dismissPushPrompt,
	getCalendar,
	itemHref,
	itemStart,
	itemTime,
	pushPromptDismissed,
	shiftWeek,
	startOfToday,
	weekStart
} from './calendar';
import type { Calendar, CalendarItem } from './types';

const game = (start: number, over: Partial<Extract<CalendarItem, { kind: 'game' }>> = {}): CalendarItem => ({
	kind: 'game',
	board_token: 'bt',
	board_title: 'Thursday padel',
	game_id: 'g1',
	start,
	end: start + 5400,
	players: ['Ana', 'Rui', 'Marta', 'Joana'],
	state: 'full',
	board_place: null,
	me: 0,
	...over
});
const event = (starts_at: number, over: Partial<Extract<CalendarItem, { kind: 'event' }>> = {}): CalendarItem => ({
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
const match = (starts_at: number, over: Partial<Extract<CalendarItem, { kind: 'match' }>> = {}): CalendarItem => ({
	kind: 'match',
	join_token: 'mt',
	starts_at,
	place: null,
	role: 'player',
	players: ['Ana', null, null, null],
	state: 'open',
	owner_name: 'Marta',
	...over
});
const cal = (items: CalendarItem[]): Calendar => ({ waiting: [], items, boards: [], open: [] });

describe('byDay', () => {
	it('by_day_groups_in_the_viewers_zone', () => {
		// 23:30 UTC on 4 Aug 2026 is 00:30 on the 5th in Lisbon (UTC+1 in summer).
		const late = game(Date.UTC(2026, 7, 4, 23, 30) / 1000);
		const noon = game(Date.UTC(2026, 7, 4, 12) / 1000, { game_id: 'g0' });
		const lisbon = byDay([noon, late], 'Europe/Lisbon');
		expect(lisbon.map((d) => d.date)).toEqual(['2026-08-04', '2026-08-05']);
		expect(lisbon[1]?.items).toEqual([late]);
		const utc = byDay([noon, late], 'UTC');
		expect(utc.map((d) => d.date)).toEqual(['2026-08-04']);
		expect(utc[0]?.items).toEqual([noon, late]);
	});

	it('keeps the order of items within a day and is empty for none', () => {
		expect(byDay([], 'UTC')).toEqual([]);
		const a = event(Date.UTC(2026, 7, 4, 9) / 1000);
		const b = match(Date.UTC(2026, 7, 4, 10) / 1000);
		expect(byDay([a, b], 'UTC')[0]?.items).toEqual([a, b]);
	});
});

describe('itemHref', () => {
	it('item_href_for_each_kind', () => {
		expect(itemHref(game(1))).toBe('/b/bt?game=g1');
		expect(itemHref(event(1))).toBe('/j/jt');
		expect(itemHref(event(1, { event_id: 'e1', role: 'organiser' }))).toBe('/events/e1');
		expect(itemHref(match(1))).toBe('/j/mt');
		expect(
			itemHref({
				kind: 'event',
				join_token: 'o1',
				name: 'x',
				event_kind: 'non_stop',
				starts_at: 1,
				place: null,
				teams: 0,
				max_teams: 8
			})
		).toBe('/j/o1');
		expect(
			itemHref({ kind: 'match', join_token: 'o2', owner_name: 'Ana', starts_at: 1, place: null, open_places: 1 })
		).toBe('/j/o2');
	});

	it('encodes what is not url safe and falls back to Home without a link', () => {
		expect(itemHref(game(1, { board_token: 'a/b', game_id: 'c d' }))).toBe('/b/a%2Fb?game=c%20d');
		expect(itemHref(event(1, { join_token: null }))).toBe('/');
	});
});

describe('itemStart', () => {
	it('reads the start of each kind', () => {
		expect(itemStart(game(5))).toBe(5);
		expect(itemStart(event(6))).toBe(6);
		expect(itemStart(match(7))).toBe(7);
	});
});

describe('weeks', () => {
	it('week_start_is_monday', () => {
		expect(weekStart('2026-10-26')).toBe('2026-10-26');
		expect(weekStart('2026-10-28')).toBe('2026-10-26');
		expect(weekStart('2026-11-01')).toBe('2026-10-26');
		expect(weekStart('2026-11-02')).toBe('2026-11-02');
	});

	it('shift_week_crosses_months', () => {
		expect(shiftWeek('2026-10-26', 1)).toBe('2026-11-02');
		expect(shiftWeek('2026-11-02', -1)).toBe('2026-10-26');
		expect(shiftWeek('2026-12-28', 1)).toBe('2027-01-04');
		expect(shiftWeek('2026-10-26', 0)).toBe('2026-10-26');
	});
});

describe('startOfToday', () => {
	it('is local midnight in the zone', () => {
		const at = Date.UTC(2026, 7, 4, 23, 30) / 1000;
		expect(startOfToday('Europe/Lisbon', at)).toBe(Date.UTC(2026, 7, 4, 23) / 1000);
		expect(startOfToday('UTC', at)).toBe(Date.UTC(2026, 7, 4) / 1000);
	});
});

describe('getCalendar', () => {
	afterEach(() => vi.unstubAllGlobals());

	it('asks for the calendar from an instant', async () => {
		const fetchMock = vi.fn(async () => new Response(JSON.stringify(cal([])), { status: 200 }));
		vi.stubGlobal('fetch', fetchMock);
		await expect(getCalendar(1234)).resolves.toEqual(cal([]));
		expect(String((fetchMock.mock.calls[0] as unknown[])[0])).toBe('/api/me/calendar?from=1234');
	});
});

describe('the push prompt', () => {
	const memory = () => {
		const data = new Map<string, string>();
		return {
			getItem: (k: string) => data.get(k) ?? null,
			setItem: (k: string, v: string) => void data.set(k, v),
			removeItem: (k: string) => void data.delete(k)
		};
	};
	const broken = {
		getItem: () => {
			throw new Error('blocked');
		},
		setItem: () => {
			throw new Error('blocked');
		},
		removeItem: () => {}
	};

	it('is remembered once dismissed', () => {
		const s = memory();
		expect(pushPromptDismissed(s)).toBe(false);
		dismissPushPrompt(s);
		expect(s.getItem(PUSH_PROMPT_KEY)).not.toBeNull();
		expect(pushPromptDismissed(s)).toBe(true);
	});

	it('survives storage that throws', () => {
		expect(pushPromptDismissed(broken)).toBe(false);
		expect(() => dismissPushPrompt(broken)).not.toThrow();
	});

	it('uses the agreed key', () => {
		expect(PUSH_PROMPT_KEY).toBe('padel-score:push-prompt-dismissed');
	});
});

describe('itemTime', () => {
	it('prints the time in the zone', () => {
		const at = Date.UTC(2026, 10, 4, 20) / 1000;
		expect(itemTime(game(at), 'UTC')).toBe('20:00–21:30');
		expect(itemTime(event(at), 'Asia/Tokyo')).toBe('05:00');
		expect(itemTime(match(at), 'UTC')).toBe('20:00');
	});
});
