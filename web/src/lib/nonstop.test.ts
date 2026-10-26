import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
import { strings } from './strings';
import {
	beepsDue,
	bestOffset,
	clockButtons,
	clockText,
	countInMs,
	createServerClock,
	isLow,
	levelOnPoints,
	markBeeps,
	markHorn,
	nonStopCourtData,
	nonStopData,
	placeText,
	playBeep,
	playHorn,
	postClock,
	msToEnd,
	remainingMs,
	shouldHorn,
	soundsDue,
	type Clock,
	type StandingRow
} from './nonstop';

const clock = (over: Partial<Clock> = {}): Clock => ({
	slot: 3,
	round: 2,
	wave: 1,
	state: 'running',
	duration_ms: 900_000,
	remaining_ms: 900_000,
	starts_at_ms: 100_000,
	ends_at_ms: 1_000_000,
	went: false,
	...over
});

afterEach(() => vi.unstubAllGlobals());

describe('server time', () => {
	it('bestOffset_picks_smallest_round_trip', () => {
		const samples = [
			{ t0: 0, t1: 300, server: 1000 },
			{ t0: 1000, t1: 1040, server: 5000 },
			{ t0: 2000, t1: 2120, server: 2500 }
		];
		expect(bestOffset(samples)).toBe(5000 - 1020);
		expect(bestOffset([])).toBeNull();
	});

	it('server_clock_syncs_and_resyncs', async () => {
		vi.useFakeTimers();
		try {
			let local = 1_000_000;
			const fetchTime = vi.fn(async () => local + 120_000);
			const c = createServerClock(fetchTime, () => local);
			expect(c.now()).toBe(local);
			await c.sync();
			expect(fetchTime).toHaveBeenCalledTimes(5);
			expect(c.now()).toBe(local + 120_000);
			await vi.advanceTimersByTimeAsync(10 * 60_000);
			expect(fetchTime).toHaveBeenCalledTimes(10);
			c.stop();
			await vi.advanceTimersByTimeAsync(20 * 60_000);
			expect(fetchTime).toHaveBeenCalledTimes(10);
		} finally {
			vi.useRealTimers();
		}
	});

	it('server_clock_times_out_a_hung_sample_and_runs_a_queued_sync', async () => {
		vi.useFakeTimers();
		try {
			let calls = 0;
			const c = createServerClock(
				() => (++calls === 1 ? new Promise<number>(() => {}) : Promise.resolve(1000)),
				() => 0
			);
			const first = c.sync();
			const second = c.sync(); // asked for while the first runs
			await vi.advanceTimersByTimeAsync(3000);
			await first;
			await second;
			expect(calls).toBe(1 + 4 + 5); // hung + 4 good, then a second pass
			expect(c.now()).toBe(1000);
			c.stop();
		} finally {
			vi.useRealTimers();
		}
	});

	it('server_clock_is_synced_once_a_sync_resolves', async () => {
		let release!: () => void;
		const gate = new Promise<void>((r) => (release = r));
		const c = createServerClock(async () => (await gate, 2000), () => 0);
		expect(c.synced()).toBe(false);
		const pending = c.sync();
		expect(c.synced()).toBe(false);
		release();
		await pending;
		expect(c.synced()).toBe(true);
		expect(c.now()).toBe(2000);
		c.stop();
		// A sync that got no sample still resolves: the phone's own time is all there is.
		const down = createServerClock(async () => Promise.reject(new Error('down')), () => 5);
		await down.sync();
		expect(down.synced()).toBe(true);
		down.stop();
	});

	it('server_clock_survives_a_failed_sync', async () => {
		const c = createServerClock(async () => Promise.reject(new Error('down')), () => 5);
		await c.sync();
		expect(c.now()).toBe(5);
		c.stop();
	});
});

describe('countdown', () => {
	it('remaining_uses_server_offset', () => {
		const local = 880_000; // 120 000 ms behind the server
		const server = local + 120_000;
		const c = clock({ starts_at_ms: 0, ends_at_ms: 1_000_000 });
		expect(remainingMs(c, server)).toBe(0);
		expect(remainingMs(c, server - 60_000)).toBe(60_000);
		expect(remainingMs(c, local)).toBe(120_000); // the unsynced local time would be wrong
	});

	it('resume_count_in_shows_what_was_left', () => {
		const c = clock({ starts_at_ms: 100_000, ends_at_ms: 502_000, remaining_ms: 402_000 });
		expect(remainingMs(c, 97_000)).toBe(402_000);
		expect(remainingMs(c, 100_000)).toBe(402_000);
		expect(remainingMs(c, 160_000)).toBe(342_000);
		expect(msToEnd(c, 600_000)).toBe(-98_000);
		expect(msToEnd(clock({ state: 'paused' }), 0)).toBe(0);
	});

	it('clockText_rounds_up', () => {
		expect(clockText(900_000)).toBe('15:00');
		expect(clockText(401_001)).toBe('06:42');
		expect(clockText(0)).toBe('00:00');
		expect(clockText(59_001)).toBe('01:00');
		expect(isLow(60_000)).toBe(true);
		expect(isLow(60_001)).toBe(false);
	});

	it('paused_and_ready_show_remaining', () => {
		for (const state of ['paused', 'ready'] as const)
			expect(
				remainingMs(clock({ state, remaining_ms: 402_000, starts_at_ms: null, ends_at_ms: null }), 5)
			).toBe(402_000);
	});

	it('done_is_zero', () => {
		expect(remainingMs(clock({ state: 'done', remaining_ms: 0, starts_at_ms: null, ends_at_ms: null }), 5)).toBe(0);
	});

	it('count_in_counts_down', () => {
		const c = clock({ starts_at_ms: 100_000, ends_at_ms: 1_000_000 });
		expect(countInMs(c, 95_800)).toBe(4200);
		expect(Math.ceil(countInMs(c, 95_800) / 1000)).toBe(5);
		expect(remainingMs(c, 95_800)).toBe(900_000);
		expect(countInMs(c, 100_000)).toBe(0);
		// A stale `now` (a 250 ms tick plus offset error) never shows a 6.
		expect(countInMs(c, 99_700)).toBe(300);
		expect(countInMs(clock({ starts_at_ms: 100_000 }), 94_700)).toBe(5000);
		expect(countInMs(clock({ starts_at_ms: 100_000 }), 0)).toBe(5000);
		expect(remainingMs(c, 100_000)).toBe(900_000);
		expect(countInMs(clock({ state: 'paused', starts_at_ms: null, ends_at_ms: null }), 0)).toBe(0);
	});
});

describe('beeps and horn', () => {
	it('beeps_once_each', () => {
		const c = clock();
		const played = new Set<string>();
		const heard: string[] = [];
		for (const left of [4500, 3400, 3000, 2900, 1900, 900, 100, -100, -300, -2000]) {
			const due = beepsDue(c, 100_000 - left, played, 'e1');
			markBeeps(c, 100_000 - left, played, 'e1');
			heard.push(...due);
		}
		expect(heard).toEqual(['beep', 'beep', 'beep', 'go']);
		// A reload after the go: nothing.
		expect(beepsDue(c, 105_000, new Set(), 'e1')).toEqual([]);
	});

	it('horn_once_per_slot', () => {
		const c = clock();
		const played = new Set<string>();
		expect(shouldHorn(c, 5_000, played, 'e1')).toBe(false);
		expect(shouldHorn(c, 0, played, 'e1')).toBe(true);
		markHorn(c, played, 'e1');
		expect(shouldHorn(c, 0, played, 'e1')).toBe(false);
		// A reload 10 s past zero.
		expect(shouldHorn(c, -10_000, new Set(), 'e1')).toBe(false);
		// Done early with time left.
		const done = clock({ state: 'done' });
		expect(shouldHorn(done, 200_000, new Set(), 'e1')).toBe(true);
	});

	it('no_sound_and_nothing_marked_until_synced', () => {
		const c = clock();
		const played = { beeps: new Set<string>(), horns: new Set<string>() };
		// 2.5 s of count-in left, and a horn due on a slot that ended just now.
		expect(soundsDue(c, 97_500, 900_000, false, played, 'e1')).toEqual({ beeps: [], horn: false });
		expect(soundsDue(c, 1_000_000, 0, false, played, 'e1')).toEqual({ beeps: [], horn: false });
		expect(played.beeps.size).toBe(0);
		expect(played.horns.size).toBe(0);
		// Synced: each plays, once.
		expect(soundsDue(c, 97_500, 900_000, true, played, 'e1')).toEqual({ beeps: ['beep'], horn: false });
		expect(soundsDue(c, 97_500, 900_000, true, played, 'e1')).toEqual({ beeps: [], horn: false });
		expect(soundsDue(c, 1_000_000, 0, true, played, 'e1')).toEqual({ beeps: [], horn: true });
		expect(soundsDue(c, 1_000_000, 0, true, played, 'e1')).toEqual({ beeps: [], horn: false });
	});

	it('playHorn_null_is_noop', () => {
		expect(() => playHorn(null)).not.toThrow();
		expect(() => playBeep(null, true)).not.toThrow();
	});

	it('playHorn_never_throws', () => {
		const ctx = { currentTime: 0, get destination(): never { throw new Error('boom'); } } as unknown as AudioContext;
		expect(() => playHorn(ctx)).not.toThrow();
		expect(() => playBeep(ctx, false)).not.toThrow();
	});
});

const settings = { match_minutes: 15, legs: 1, order: 'listed' };
const row = {
	team_id: 't1',
	place: 1,
	place_to: 1,
	played: 2,
	won: 2,
	drawn: 0,
	lost: 0,
	games_won: 12,
	points: 6
};

describe('guards', () => {
	it('guards_reject_malformed', () => {
		expect(nonStopData({})).toBeNull();
		expect(nonStopData({ clock: 3 })).toBeNull();
		expect(nonStopData(null)).toBeNull();
		expect(nonStopCourtData({ clock: 3, rounds: 1, waves: 1 })).toBeNull();
		const full = {
			settings,
			rounds: 7,
			waves: 2,
			clock: clock(),
			slots: [{ slot: 1, round: 1, wave: 1, state: 'done', matches: ['m1', 'm2'] }],
			standings: [row]
		};
		expect(nonStopData(full)).toEqual(full);
		expect(nonStopData({ ...full, clock: null })?.clock).toBeNull();
		expect(nonStopData({ ...full, standings: [{ ...row, points: 'x' }] })).toBeNull();
		expect(nonStopData({ ...full, settings: { ...settings, order: 'sorted' } })).toBeNull();
		expect(nonStopData({ ...full, clock: { ...clock(), state: 'weird' } })).toBeNull();
		expect(nonStopData({ ...full, clock: { ...clock(), went: 'yes' } })).toBeNull();
		expect(nonStopCourtData({ clock: clock(), rounds: 7, waves: 2 })).toEqual({
			clock: clock(),
			rounds: 7,
			waves: 2
		});
		expect(nonStopCourtData({ clock: null, rounds: 0, waves: 0 })?.clock).toBeNull();
	});
});

describe('end-now copy', () => {
	it('names_what_ends', () => {
		const t = strings.nonStop;
		expect(t.endNowConfirm(t.slotText(2, 1, 2))).toContain('End round 2, wave 1 now?');
		expect(t.endNowConfirm(t.slotText(2, 1, 1))).toContain('End round 2 now?');
	});
});

describe('postClock', () => {
	it('posts_the_action_and_slot', async () => {
		const fetchMock = vi.fn(async () => new Response('{"id":"e1"}', { status: 200 }));
		vi.stubGlobal('fetch', fetchMock);
		await postClock('e1', 'start', 3);
		const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
		expect(url).toBe('/api/events/e1/clock');
		expect(init.method).toBe('POST');
		expect(JSON.parse(init.body as string)).toEqual({ action: 'start', slot: 3 });
	});

	it('maps_409_to_ApiError', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => new Response('{"error":"clock_busy"}', { status: 409 }))
		);
		await expect(postClock('e1', 'start', 3)).rejects.toMatchObject({ status: 409, code: 'clock_busy' });
		await expect(postClock('e1', 'start', 3)).rejects.toBeInstanceOf(ApiError);
	});
});

describe('clockButtons', () => {
	it('clockButton_for_each_state', () => {
		expect(clockButtons(clock({ state: 'ready' }))).toEqual(['start']);
		expect(clockButtons(clock({ state: 'running' }))).toEqual(['pause', 'end']);
		expect(clockButtons(clock({ state: 'paused' }))).toEqual(['resume', 'end']);
		expect(clockButtons(clock({ state: 'done' }))).toEqual([]);
		expect(clockButtons(null)).toEqual([]);
	});
});

describe('placeText', () => {
	it('shows_a_range_once_for_a_shared_place', () => {
		expect(placeText({ place: 1, place_to: 1 })).toBe('1');
		expect(placeText({ place: 2, place_to: 3 })).toBe('2–3');
	});
});

describe('levelOnPoints', () => {
	it('marks_teams_level_on_points_even_when_places_differ', () => {
		const row = (team_id: string, place: number, points: number): StandingRow => ({
			team_id, place, place_to: place, played: 3, won: 0, drawn: 0, lost: 0, games_won: 0, points
		});
		const rows = [row('a', 1, 9), row('b', 2, 4), row('c', 3, 4), row('d', 4, 1)];
		expect([...levelOnPoints(rows)].sort()).toEqual(['b', 'c']);
		expect(levelOnPoints([]).size).toBe(0);
	});
});

describe('courtLine', () => {
	it('courtLine_text', () => {
		expect(strings.nonStop.courtLine('Court 2', 3, 7)).toBe('Round 3 of 7, Court 2');
		expect(strings.nonStop.courtLine('Court 2', 3, 7, 1, 1)).toBe('Round 3 of 7, Court 2');
	});

	it('courtLine_text_with_waves', () => {
		expect(strings.nonStop.courtLine('Court 2', 3, 7, 2, 2)).toBe('Round 3 of 7, wave 2, Court 2');
	});

	it('courtLine_shows_the_name_as_given', () => {
		expect(strings.nonStop.courtLine('Centre', 1, 7)).toBe('Round 1 of 7, Centre');
	});
});
