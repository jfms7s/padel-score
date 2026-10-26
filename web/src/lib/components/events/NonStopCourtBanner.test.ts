// Server-rendered: the banner's first paint for each clock state. $effect does not run here, so
// nothing ticks; the countdown maths has its own tests in nonstop.test.ts.
import { render } from 'svelte/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Clock, SlotState } from '#lib/nonstop.ts';
import { strings } from '#lib/strings.ts';
import type { CourtState } from '#lib/types.ts';
import NonStopCourtBanner from './NonStopCourtBanner.svelte';

const t = strings.nonStop;
const NOW = 1_800_000_000_000;

const clock = (state: SlotState, over: Partial<Clock> = {}): Clock => ({
	slot: 1,
	round: 2,
	wave: 1,
	state,
	duration_ms: 600_000,
	remaining_ms: 600_000,
	starts_at_ms: null,
	ends_at_ms: null,
	went: false,
	...over
});

const court = (c: Clock | null, name = 'Court 1', waves = 1): CourtState => ({
	event: { id: 'e1', name: 'Night', kind: 'non_stop', status: 'running' },
	court: { id: 'c1', name },
	current: null,
	next: null,
	kind_data: { clock: c, rounds: 3, waves }
});

/** The text of the banner's HTML, tags dropped. */
const html = (state: CourtState, viewer = false): string =>
	render(NonStopCourtBanner, { props: { court: state, viewer } })
		.body.replace(/<[^>]*>/g, ' ')
		.replace(/\s+/g, ' ');

describe('NonStopCourtBanner', () => {
	beforeEach(() => {
		vi.useFakeTimers();
		vi.setSystemTime(NOW);
	});
	afterEach(() => vi.useRealTimers());

	it('renders nothing without a clock or court data', () => {
		expect(html(court(null)).trim()).toBe('');
		expect(html({ ...court(null), kind_data: { nope: 1 } }).trim()).toBe('');
	});

	it('shows the court line with the name as given', () => {
		expect(html(court(clock('ready')))).toContain('Round 2 of 3, Court 1');
		expect(html(court(clock('ready'), 'Centre', 2))).toContain('Round 2 of 3, wave 1, Centre');
	});

	it('ready: waiting for the clock, with the practice note', () => {
		const text = html(court(clock('ready')));
		expect(text).toContain(t.waitingForClock);
		expect(text).toContain(t.practiceNote);
		expect(text).not.toContain(t.paused);
	});

	it('paused before the go: one combined status and the practice note', () => {
		const text = html(court(clock('paused', { remaining_ms: 600_000 })));
		expect(text).toContain(t.pausedBeforeGo);
		expect(text).toContain('10:00');
		expect(text).toContain(t.practiceNote);
	});

	it('count-in: the digit', () => {
		const text = html(court(clock('running', { starts_at_ms: NOW + 3000, ends_at_ms: NOW + 603_000 })));
		expect(text).toContain(t.countIn);
		expect(text).toContain(' 3 ');
		// A running clock isn't "before the go": no practice note.
		expect(text).not.toContain(t.practiceNote);
	});

	it('running after the go: the countdown, no status and no practice note', () => {
		const text = html(
			court(
				clock('running', { went: true, starts_at_ms: NOW - 60_000, ends_at_ms: NOW + 90_000, remaining_ms: 150_000 })
			)
		);
		expect(text).toContain('01:30');
		expect(text).not.toContain(t.paused);
		expect(text).not.toContain(t.practiceNote);
	});

	it('paused after the go: the time left and "Paused" only', () => {
		const text = html(court(clock('paused', { went: true, remaining_ms: 125_000 })));
		expect(text).toContain('02:05');
		expect(text).toContain(t.paused);
		expect(text).not.toContain(t.pausedBeforeGo);
		expect(text).not.toContain(t.practiceNote);
	});

	it('low time is marked, a longer time is not', () => {
		const low = render(NonStopCourtBanner, {
			props: { court: court(clock('running', { went: true, starts_at_ms: NOW - 1000, ends_at_ms: NOW + 30_000 })) }
		}).body;
		const long = render(NonStopCourtBanner, {
			props: { court: court(clock('running', { went: true, starts_at_ms: NOW - 1000, ends_at_ms: NOW + 300_000 })) }
		}).body;
		expect(low).toMatch(/class="[^"]*\blow\b/);
		expect(long).not.toMatch(/class="[^"]*\blow\b/);
	});

	it('done: time is up, no countdown', () => {
		const text = html(court(clock('done', { went: true, remaining_ms: 0 })));
		expect(text).toContain(t.timeUp);
		expect(text).not.toContain('00:00');
	});
});
