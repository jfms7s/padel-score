// Non-stop events (phase 8): the wire types, the calls, the shared server-time clock, and the
// pure countdown, beep and horn maths the court phone, the coordinator and the viewers share.
//
// The server owns the clock. A page only measures its offset to server time (`GET /api/time`)
// and counts down locally from the `Clock` a frame carries, so nothing here ticks on the wire.

import { accountRequest, plainRequest } from './api';
import { array, atLeast, int, nullable, oneOf, str, type Check } from './guards';
import type { EventDetail } from './types';

/** The count-in before a clock moves (server: `COUNT_IN_MS`). */
export const COUNT_IN_MS = 5000;
/** The countdown turns red at this much time left or less. */
export const LOW_MS = 60_000;
/** How late after zero a horn or a go beep still plays (a reload later stays quiet). */
const LATE_MS = 3000;
const BEEP_HZ = 880;

// ---- wire types --------------------------------------------------------------------------

export interface NonStopSettings {
	match_minutes: number;
	legs: number;
	order: 'listed' | 'random';
}

export type SlotState = 'ready' | 'running' | 'paused' | 'done';

export interface Clock {
	slot: number;
	round: number;
	wave: number;
	state: SlotState;
	duration_ms: number;
	/** Running: as of the read (the full time during the count-in). */
	remaining_ms: number;
	/** Running only; in the future during the count-in. */
	starts_at_ms: number | null;
	ends_at_ms: number | null;
}

export interface SlotView {
	slot: number;
	round: number;
	wave: number;
	state: SlotState;
	matches: string[];
}

export interface StandingRow {
	team_id: string;
	place: number;
	place_to: number;
	played: number;
	won: number;
	drawn: number;
	lost: number;
	games_won: number;
	points: number;
}

/** `kind_data` for the coordinator and for spectators. */
export interface NonStopData {
	settings: NonStopSettings;
	rounds: number;
	waves: number;
	clock: Clock | null;
	slots: SlotView[];
	standings: StandingRow[];
}

/** `kind_data` for a court. */
export interface NonStopCourtData {
	clock: Clock | null;
	rounds: number;
	waves: number;
}

// ---- guards ------------------------------------------------------------------------------

const slotState = oneOf('ready', 'running', 'paused', 'done');
const settingsCheck = atLeast({ match_minutes: int, legs: int, order: oneOf('listed', 'random') });
const clockCheck = atLeast({
	slot: int,
	round: int,
	wave: int,
	state: slotState,
	duration_ms: int,
	remaining_ms: int,
	starts_at_ms: nullable(int),
	ends_at_ms: nullable(int)
});
const slotCheck = atLeast({ slot: int, round: int, wave: int, state: slotState, matches: array(str) });
const standingCheck = atLeast({
	team_id: str,
	place: int,
	place_to: int,
	played: int,
	won: int,
	drawn: int,
	lost: int,
	games_won: int,
	points: int
});
const dataCheck: Check = atLeast({
	settings: settingsCheck,
	rounds: int,
	waves: int,
	clock: nullable(clockCheck),
	slots: array(slotCheck),
	standings: array(standingCheck)
});
const courtCheck: Check = atLeast({ clock: nullable(clockCheck), rounds: int, waves: int });

/** The coordinator's or a spectator's `kind_data`; `null` when it isn't one (never throws). */
export const nonStopData = (v: unknown): NonStopData | null => (dataCheck(v) ? (v as NonStopData) : null);

/** A court's `kind_data`; `null` when it isn't one (never throws). */
export const nonStopCourtData = (v: unknown): NonStopCourtData | null =>
	courtCheck(v) ? (v as NonStopCourtData) : null;

// ---- calls -------------------------------------------------------------------------------

/** The server's clock in unix milliseconds. No credential. */
export async function getServerTime(): Promise<number> {
	const body = await plainRequest<{ now_ms: number }>('/api/time');
	return body.now_ms;
}

/**
 * Start, pause, resume or end the clock of `slot` (the slot the screen showed, so a stale tap is
 * refused). 409 `not_next_slot`, `clock_busy`, `previous_unfinished`, `clock_not_running`,
 * `clock_not_paused`, `event_not_running`, `not_non_stop`.
 */
export function postClock(
	eventId: string,
	action: 'start' | 'pause' | 'resume' | 'end',
	slot: number
): Promise<EventDetail> {
	return accountRequest(`/api/events/${encodeURIComponent(eventId)}/clock`, {
		signedIn: true,
		method: 'POST',
		body: { action, slot }
	});
}

// ---- server time -------------------------------------------------------------------------

export interface TimeSample {
	t0: number;
	t1: number;
	server: number;
}

/** The offset (server minus local) from the sample with the shortest round trip, or null. */
export function bestOffset(samples: readonly TimeSample[]): number | null {
	let best: TimeSample | null = null;
	for (const s of samples) if (best === null || s.t1 - s.t0 < best.t1 - best.t0) best = s;
	return best === null ? null : best.server - (best.t0 + best.t1) / 2;
}

const SAMPLES = 5;
const RESYNC_MS = 10 * 60_000;
const SAMPLE_TIMEOUT_MS = 3000;

export interface ServerClock {
	/** Server time in unix ms; the local time until the first sync lands. */
	now(): number;
	/** Take samples and update the offset; starts the 10-minute re-sync. Never throws. */
	sync(): Promise<void>;
	stop(): void;
}

export function createServerClock(
	fetchTime: () => Promise<number> = getServerTime,
	now: () => number = () => Date.now()
): ServerClock {
	let offset = 0;
	let timer: ReturnType<typeof setInterval> | null = null;
	let syncing: Promise<void> | null = null;
	let again = false;

	/** One sample, given up on after SAMPLE_TIMEOUT_MS so a hung fetch can't block later syncs. */
	function sample(): Promise<number> {
		return new Promise((resolve, reject) => {
			const t = setTimeout(() => reject(new Error('time sample timed out')), SAMPLE_TIMEOUT_MS);
			fetchTime().then(
				(v) => (clearTimeout(t), resolve(v)),
				(e) => (clearTimeout(t), reject(e))
			);
		});
	}

	async function measure(): Promise<void> {
		const samples: TimeSample[] = [];
		for (let i = 0; i < SAMPLES; i++) {
			const t0 = now();
			try {
				const server = await sample();
				samples.push({ t0, t1: now(), server });
			} catch {
				// A lost sample just leaves fewer to choose from.
			}
		}
		const best = bestOffset(samples);
		if (best !== null) offset = best;
	}

	function sync(): Promise<void> {
		timer ??= setInterval(() => void sync(), RESYNC_MS);
		if (syncing !== null) {
			// A sync asked for while one runs (a reconnect) measures once more afterwards.
			again = true;
			return syncing;
		}
		syncing = (async () => {
			try {
				do {
					again = false;
					await measure();
				} while (again);
			} finally {
				syncing = null;
			}
		})();
		return syncing;
	}

	return {
		now: () => now() + offset,
		sync,
		stop() {
			if (timer !== null) clearInterval(timer);
			timer = null;
		}
	};
}

let shared: ServerClock | null = null;

/** The one clock every non-stop component on the page shares (created on first use). */
export function serverClock(): ServerClock {
	return (shared ??= createServerClock());
}

// ---- countdown ---------------------------------------------------------------------------

/** Milliseconds of count-in left; 0 outside one. The screen shows `Math.ceil(ms / 1000)`. */
export function countInMs(clock: Clock, serverNow: number): number {
	if (clock.state !== 'running' || clock.starts_at_ms === null) return 0;
	return Math.max(0, clock.starts_at_ms - serverNow);
}

/** Time left on the clock; during the count-in the full time. */
export function remainingMs(clock: Clock, serverNow: number): number {
	switch (clock.state) {
		case 'running':
			// Clamp to this run's length (a resume runs only what was left), so the count-in
			// shows the time the run will have, not the full match.
			return clock.ends_at_ms === null
				? clock.remaining_ms
				: Math.min(
						clock.starts_at_ms === null ? clock.remaining_ms : clock.ends_at_ms - clock.starts_at_ms,
						Math.max(0, clock.ends_at_ms - serverNow)
					);
		case 'done':
			return 0;
		default:
			return clock.remaining_ms;
	}
}

/** Unclamped time to the deadline (negative once past it); 0 unless running. For `shouldHorn`. */
export function msToEnd(clock: Clock, serverNow: number): number {
	return clock.state === 'running' && clock.ends_at_ms !== null ? clock.ends_at_ms - serverNow : 0;
}

/** `mm:ss`, the seconds rounded up so the clock reads 00:00 only at zero. */
export function clockText(ms: number): string {
	const total = Math.ceil(Math.max(0, ms) / 1000);
	const mm = String(Math.floor(total / 60)).padStart(2, '0');
	const ss = String(total % 60).padStart(2, '0');
	return `${mm}:${ss}`;
}

export const isLow = (ms: number): boolean => ms <= LOW_MS;

// ---- beeps and horn ----------------------------------------------------------------------

export type Beep = 'beep' | 'go';

/** The one beep (if any) the count-in is due to play now, with its once-only key. */
function beepNow(clock: Clock, serverNow: number, eventId: string): { kind: Beep; key: string } | null {
	if (clock.state !== 'running' || clock.starts_at_ms === null) return null;
	const left = clock.starts_at_ms - serverNow;
	const base = `${eventId}:${clock.slot}:${clock.starts_at_ms}`;
	if (left <= 0) return left > -LATE_MS ? { kind: 'go', key: `${base}:go` } : null;
	if (left > 3000) return null;
	const n = Math.ceil(left / 1000); // 3, 2 or 1 seconds left
	return { kind: 'beep', key: `${base}:${n}` };
}

/** The beeps due now: one at 3, 2 and 1 s left of the count-in, `go` at 0, each once. */
export function beepsDue(
	clock: Clock,
	serverNow: number,
	played: ReadonlySet<string>,
	eventId: string
): Beep[] {
	const due = beepNow(clock, serverNow, eventId);
	return due !== null && !played.has(due.key) ? [due.kind] : [];
}

/** Record the beeps `beepsDue` returned for these arguments as played. */
export function markBeeps(clock: Clock, serverNow: number, played: Set<string>, eventId: string): void {
	const due = beepNow(clock, serverNow, eventId);
	if (due !== null) played.add(due.key);
}

const hornKey = (clock: Clock, eventId: string) => `${eventId}:${clock.slot}`;

/**
 * Whether to sound the horn: once per `eventId:slot`, when the clock has just reached zero
 * (`remaining` is `msToEnd(clock, serverNow)`, not the clamped `remainingMs`: 0 down to −3 s counts, a reload later
 * stays quiet), or when a frame turns the slot `done` while `remaining` (the last local value)
 * still had time on it (End now).
 */
export function shouldHorn(
	clock: Clock,
	remaining: number,
	played: ReadonlySet<string>,
	eventId: string
): boolean {
	if (played.has(hornKey(clock, eventId))) return false;
	if (clock.state === 'done') return remaining > 0;
	return clock.state === 'running' && remaining <= 0 && remaining > -LATE_MS;
}

export function markHorn(clock: Clock, played: Set<string>, eventId: string): void {
	played.add(hornKey(clock, eventId));
}

function tone(ctx: AudioContext, type: OscillatorType, hz: number, start: number, secs: number, peak: number) {
	const osc = ctx.createOscillator();
	const gain = ctx.createGain();
	osc.type = type;
	osc.frequency.value = hz;
	gain.gain.setValueAtTime(0.0001, start);
	gain.gain.exponentialRampToValueAtTime(peak, start + 0.02);
	gain.gain.setValueAtTime(peak, start + secs - 0.05);
	gain.gain.exponentialRampToValueAtTime(0.0001, start + secs);
	osc.connect(gain).connect(ctx.destination);
	osc.start(start);
	osc.stop(start + secs);
}

/** A short beep (0.15 s), or a long one (0.5 s) for the go. Never throws. */
export function playBeep(ctx: AudioContext | null, long: boolean): void {
	if (ctx === null) return;
	try {
		tone(ctx, 'sine', BEEP_HZ, ctx.currentTime, long ? 0.5 : 0.15, 0.3);
	} catch {
		// No sound is better than a broken clock.
	}
}

/** The horn: two square-wave tones, 440 Hz then 330 Hz, 0.6 s each. Never throws. */
export function playHorn(ctx: AudioContext | null): void {
	if (ctx === null) return;
	try {
		const t = ctx.currentTime;
		tone(ctx, 'square', 440, t, 0.6, 0.2);
		tone(ctx, 'square', 330, t + 0.6, 0.6, 0.2);
	} catch {
		// No sound is better than a broken clock.
	}
}
