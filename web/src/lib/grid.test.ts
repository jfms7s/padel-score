import { describe, expect, it } from 'vitest';
import { localMidnight, timeText, weekDays } from './dates';
import {
	dayCells,
	freeAt,
	heatMap,
	layoutGames,
	mergedRanges,
	orientation,
	othersFreeCount,
	rangeFromCells,
	shade,
	slotsOfDay
} from './grid';
import type { BoardView, PieceOut } from './types';

const H = 3600;
const T0 = Date.UTC(2026, 10, 2) / 1000; // Monday 2026-11-02 00:00 UTC

const piece = (user_id: string, start: number, end: number, created_at = 0): PieceOut => ({
	user_id,
	start,
	end,
	created_at
});

function view(over: Partial<BoardView> = {}): BoardView {
	return {
		board: {
			id: 'b',
			token: 't',
			title: 'Thursday padel',
			place: null,
			time_zone: 'UTC',
			starts_at: T0,
			ends_at: T0 + 7 * 86400,
			creator_user_id: 'ana',
			creator_name: 'Ana',
			over: false,
			allow_guests: false
		},
		me: null,
		members: [
			{ user_id: 'ana', name: 'Ana', guest: false, joined_at: 10 },
			{ user_id: 'rui', name: 'Rui', guest: false, joined_at: 20 },
			{ user_id: 'marta', name: 'Marta', guest: false, joined_at: 30 }
		],
		pieces: [],
		games: [],
		requests: [],
		...over
	};
}

describe('slotsOfDay', () => {
	it('gives 48 half hours on a normal day', () => {
		const slots = slotsOfDay({ start: T0, end: T0 + 86400 });
		expect(slots).toHaveLength(48);
		expect(slots[0]).toBe(T0);
		expect(slots[47]).toBe(T0 + 86400 - 1800);
	});

	it('gives 50 on the 25-hour Sunday and 46 on the 23-hour one', () => {
		const tz = 'Europe/Lisbon';
		const fall = weekDays(localMidnight('2026-10-19', tz), localMidnight('2026-10-26', tz), tz);
		expect(slotsOfDay(fall[6]!)).toHaveLength(50);
		const spring = weekDays(localMidnight('2026-03-23', tz), localMidnight('2026-03-30', tz), tz);
		expect(slotsOfDay(spring[6]!)).toHaveLength(46);
	});
});

describe('dayCells', () => {
	it('puts each half hour on its clock row', () => {
		const cells = dayCells({ start: T0, end: T0 + 86400 }, 'UTC');
		expect(cells).toHaveLength(48);
		expect(cells[14]).toEqual({ t: T0 + 7 * H, row: 14 });
	});

	it('keeps the clock rows aligned across the end of summer time, the repeated hour on extra rows', () => {
		const tz = 'Europe/Lisbon';
		const sunday = weekDays(localMidnight('2026-10-19', tz), localMidnight('2026-10-26', tz), tz)[6]!;
		const cells = dayCells(sunday, tz);
		expect(cells).toHaveLength(50);
		const rows = cells.map((c) => c.row);
		expect(new Set(rows).size).toBe(50);
		// 04:00 on Sunday is the same row as 04:00 on any other day.
		const at4 = cells.find((c) => c.t === localMidnight('2026-10-25', tz) + 5 * H);
		expect(at4?.row).toBe(8);
		// The second 01:00 and 01:30 sit after the 48 clock rows.
		expect(rows.filter((r) => r >= 48).sort()).toEqual([48, 49]);
	});

	it('leaves the skipped hour out on the 23-hour day', () => {
		const tz = 'Europe/Lisbon';
		const sunday = weekDays(localMidnight('2026-03-23', tz), localMidnight('2026-03-30', tz), tz)[6]!;
		const rows = dayCells(sunday, tz).map((c) => c.row);
		expect(rows).toHaveLength(46);
		expect(rows).not.toContain(2);
		expect(rows).not.toContain(3);
	});
});

describe('a viewer whose zone is not a whole half hour from the board', () => {
	it('keeps every cell on the board s half-hour grid (Kathmandu viewing a Lisbon board)', () => {
		const board = 'Europe/Lisbon';
		const viewer = 'Asia/Kathmandu';
		const startsAt = localMidnight('2026-11-02', board) + 18 * H;
		const endsAt = localMidnight('2026-11-09', board);
		const days = weekDays(startsAt, endsAt, viewer);
		for (const day of days) {
			const cells = dayCells(day, viewer, startsAt);
			expect(cells.length).toBeGreaterThanOrEqual(47);
			for (const c of cells) {
				// What the server checks of an add: (t - board.starts_at) % 1800 == 0.
				expect((((c.t - startsAt) % 1800) + 1800) % 1800).toBe(0);
				expect(c.t).toBeGreaterThanOrEqual(day.start);
				expect(c.t).toBeLessThan(day.end);
			}
			expect(new Set(cells.map((c) => c.row)).size).toBe(cells.length);
		}
		// Without the anchor the cells are 15 minutes off the board's grid.
		expect((dayCells(days[1]!, viewer)[0]!.t - startsAt) % 1800).not.toBe(0);
		// The labels show the viewer's clock.
		expect(timeText(dayCells(days[1]!, viewer, startsAt)[1]!.t, viewer)).toMatch(/:(15|45)$/);
	});

	it('changes nothing when the zones agree', () => {
		const tz = 'Europe/Lisbon';
		const day = weekDays(localMidnight('2026-10-19', tz), localMidnight('2026-10-26', tz), tz)[6]!;
		expect(dayCells(day, tz, localMidnight('2026-10-19', tz) + 18 * H)).toEqual(dayCells(day, tz));
	});
});

describe('a week across a midnight spring-forward', () => {
	it('puts Saturday 23:00 in Saturday s column and none of it in Sunday s', () => {
		const tz = 'Atlantic/Azores';
		const days = weekDays(localMidnight('2027-03-22', tz), localMidnight('2027-03-29', tz), tz);
		expect(days).toHaveLength(7);
		const sat = dayCells(days[5]!, tz);
		const sun = dayCells(days[6]!, tz);
		const at2300 = days[5]!.end - 3600;
		expect(timeText(at2300, tz)).toBe('23:00');
		expect(sat.some((c) => c.t === at2300)).toBe(true);
		expect(sun.some((c) => c.t === at2300)).toBe(false);
		expect(sun).toHaveLength(46);
	});
});

describe('rangeFromCells', () => {
	it('is order free and ends after the later cell', () => {
		const a = T0 + 20 * H;
		const b = T0 + 21 * H;
		expect(rangeFromCells(b, a)).toEqual(rangeFromCells(a, b));
		expect(rangeFromCells(a, b)).toEqual({ start: a, end: b + 1800 });
		expect(rangeFromCells(a, a)).toEqual({ start: a, end: a + 1800 });
	});
});

describe('shade', () => {
	it('is none, then some for 1-2, then game for 3 or more', () => {
		expect(shade(0)).toBe('none');
		expect(shade(1)).toBe('some');
		expect(shade(2)).toBe('some');
		expect(shade(3)).toBe('game');
		expect(shade(9)).toBe('game');
	});
});

describe('othersFreeCount and heatMap', () => {
	const pieces = [
		piece('ana', T0, T0 + 4 * H),
		piece('rui', T0 + H, T0 + 2 * H),
		piece('rui', T0 + 90 * 60, T0 + 3 * H), // overlaps itself: still one person
		piece('marta', T0 + H, T0 + 2 * H)
	];

	it('counts the other people free at a half hour, once each', () => {
		expect(othersFreeCount(pieces, 'ana', T0 + H)).toBe(2);
		expect(othersFreeCount(pieces, 'ana', T0 + 90 * 60)).toBe(2);
		expect(othersFreeCount(pieces, 'ana', T0 + 2 * H)).toBe(1);
		expect(othersFreeCount(pieces, 'ana', T0 + 3 * H)).toBe(0);
		expect(othersFreeCount(pieces, null, T0)).toBe(1);
		expect(othersFreeCount(pieces, 'rui', T0)).toBe(1);
	});

	it('heatMap agrees with othersFreeCount on every half hour', () => {
		const map = heatMap(pieces, 'ana');
		for (let t = T0 - 1800; t <= T0 + 5 * H; t += 1800) {
			expect(map.get(t) ?? 0).toBe(othersFreeCount(pieces, 'ana', t));
		}
	});
});

describe('free counts with games', () => {
	const pieces = [piece('ana', T0, T0 + 4 * H, 1), piece('rui', T0, T0 + 4 * H, 2), piece('marta', T0, T0 + 4 * H, 3)];
	const games = [{ start: T0 + H, end: T0 + 2 * H, players: [{ user_id: 'rui', name: 'Rui', guest: false }] }];

	it('does not count someone who is in a game then, and agrees with freeAt', () => {
		expect(othersFreeCount(pieces, 'ana', T0 + H, games)).toBe(1);
		expect(othersFreeCount(pieces, 'ana', T0 + 2 * H, games)).toBe(2);
		const heat = heatMap(pieces, 'ana', games);
		const v = view({
			pieces,
			games: games.map((g, i) => ({
				...g,
				id: `g${i}`,
				version: 1,
				moved: false,
				state: 'short' as const,
				line: [],
				match_spectator_token: null
			}))
		});
		for (let t = T0; t < T0 + 4 * H; t += 1800) {
			const listed = freeAt(v, t).filter((m) => m.user_id !== 'ana').length;
			expect(heat.get(t) ?? 0).toBe(listed);
			expect(othersFreeCount(pieces, 'ana', t, games)).toBe(listed);
		}
	});
});

describe('heat in a zone with a quarter-hour offset', () => {
	it('lines up with the board s grid, not the epoch s', () => {
		// Kathmandu is UTC+5:45: local midnight is 18:15 UTC the day before.
		const tz = 'Asia/Kathmandu';
		const start = localMidnight('2026-11-02', tz);
		expect(start % 1800).toBe(900);
		const pieces = [piece('rui', start + 20 * H, start + 22 * H), piece('marta', start + 21 * H, start + 23 * H)];
		const map = heatMap(pieces, 'ana');
		for (let t = start + 19 * H; t < start + 24 * H; t += 1800) {
			expect(map.get(t) ?? 0).toBe(othersFreeCount(pieces, 'ana', t));
		}
		expect(map.get(start + 21 * H)).toBe(2);
	});
});

describe('layoutGames', () => {
	const g = (id: string, start: number, end: number, who: string[] = []) => ({
		id,
		start,
		end,
		players: who.map((user_id) => ({ user_id, name: user_id, guest: false }))
	});
	const lanes = (list: ReturnType<typeof layoutGames>) =>
		Object.fromEntries(list.map((l) => [l.id, [l.lane, l.lanes]]));

	it('leaves a lone game the whole column', () => {
		expect(lanes(layoutGames([g('a', T0, T0 + H)], null))).toEqual({ a: [0, 1] });
	});

	it('splits the column between two simultaneous games, the viewer s own first', () => {
		const out = layoutGames([g('a', T0, T0 + H, ['x']), g('b', T0, T0 + H, ['me'])], 'me');
		expect(lanes(out)).toEqual({ b: [0, 2], a: [1, 2] });
		expect(out.map((l) => l.id)).toEqual(['b', 'a']);
	});

	it('splits among three, then by start and by the order they came in', () => {
		const out = layoutGames([g('c', T0, T0 + H), g('a', T0, T0 + H), g('b', T0 - 1800, T0 + H)], null);
		expect(lanes(out)).toEqual({ b: [0, 3], c: [1, 3], a: [2, 3] });
	});

	it('gives a game that only partly overlaps its own lane and reuses a free lane after', () => {
		const out = layoutGames([g('a', T0, T0 + 2 * H), g('b', T0 + H, T0 + 3 * H), g('c', T0 + 2 * H, T0 + 4 * H)], null);
		// a and b overlap, b and c overlap: one cluster of 2 lanes; c reuses a's lane.
		expect(lanes(out)).toEqual({ a: [0, 2], b: [1, 2], c: [0, 2] });
	});

	it('does not narrow games that do not overlap', () => {
		const out = layoutGames([g('a', T0, T0 + H), g('b', T0 + H, T0 + 2 * H)], null);
		expect(lanes(out)).toEqual({ a: [0, 1], b: [0, 1] });
	});
});

describe('mergedRanges', () => {
	it('joins touching and overlapping pieces of one member', () => {
		const pieces = [
			piece('ana', T0 + 4 * H, T0 + 5 * H),
			piece('ana', T0, T0 + H),
			piece('ana', T0 + H, T0 + 2 * H),
			piece('ana', T0 + 90 * 60, T0 + 150 * 60),
			piece('rui', T0, T0 + 10 * H)
		];
		expect(mergedRanges(pieces, 'ana')).toEqual([
			{ start: T0, end: T0 + 150 * 60 },
			{ start: T0 + 4 * H, end: T0 + 5 * H }
		]);
		expect(mergedRanges(pieces, 'nobody')).toEqual([]);
	});
});

describe('freeAt', () => {
	it('orders by the covering piece created_at before joined_at', () => {
		const v = view({
			pieces: [
				piece('ana', T0, T0 + 4 * H, 500),
				piece('rui', T0, T0 + 4 * H, 100),
				piece('marta', T0, T0 + 4 * H, 100)
			]
		});
		// Rui and Marta tie on created_at: Rui joined first. Ana joined first of all but was late.
		expect(freeAt(v, T0 + H).map((m) => m.user_id)).toEqual(['rui', 'marta', 'ana']);
	});

	it('uses the earliest of a member s covering pieces and ignores pieces that do not cover', () => {
		const v = view({
			pieces: [
				piece('ana', T0, T0 + 2 * H, 900),
				piece('ana', T0 + H, T0 + 3 * H, 50),
				piece('rui', T0 + 2 * H, T0 + 3 * H, 10)
			]
		});
		expect(freeAt(v, T0 + 90 * 60).map((m) => m.user_id)).toEqual(['ana']);
		expect(freeAt(v, T0 + 2 * H).map((m) => m.user_id)).toEqual(['rui', 'ana']);
		expect(freeAt(v, T0 + 3 * H)).toEqual([]);
	});

	it('leaves out members who are in a game at that time', () => {
		const v = view({
			pieces: [piece('ana', T0, T0 + 4 * H, 1), piece('rui', T0, T0 + 4 * H, 2)],
			games: [
				{
					id: 'g',
					start: T0 + H,
					end: T0 + 2 * H + 1800,
					version: 1,
					moved: false,
					state: 'short',
					players: [{ user_id: 'ana', name: 'Ana', guest: false }],
					line: [],
					match_spectator_token: null
				}
			]
		});
		expect(freeAt(v, T0 + H).map((m) => m.user_id)).toEqual(['rui']);
		expect(freeAt(v, T0 + 3 * H).map((m) => m.user_id)).toEqual(['ana', 'rui']);
	});
});

describe('orientation', () => {
	it('turns the grid into columns below 768 px', () => {
		expect(orientation(390)).toBe('columns');
		expect(orientation(767)).toBe('columns');
		expect(orientation(768)).toBe('rows');
		expect(orientation(1280)).toBe('rows');
	});
});
