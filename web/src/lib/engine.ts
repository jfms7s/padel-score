// The browser's copy of the scoring engine (crates/scoring-wasm), for scoring while offline.
// The bindings are generated into ./wasm by scripts/build-wasm.sh.

import init, { initSync, replay } from '#lib/wasm/padel_scoring_wasm.js';
import type { Event, Format, MatchView, ReplayResult, Start } from './types';

let loading: Promise<void> | null = null;
let ready = false;

/** Load the engine. Safe to call more than once; a failed load can be retried. */
export function initEngine(): Promise<void> {
	loading ??= init().then(
		() => {
			ready = true;
		},
		(e: unknown) => {
			loading = null;
			throw e;
		}
	);
	return loading;
}

/** Load the engine from wasm bytes already in hand (tests, workers). */
export function initEngineSync(bytes: BufferSource): void {
	initSync({ module: bytes });
	ready = true;
}

export function engineReady(): boolean {
	return ready;
}

/** Replay `events` with the engine, as the server does. Call `initEngine` first. */
export function replayLocal(format: Format, start: Start, events: Event[]): ReplayResult {
	if (!ready) throw new Error('the scoring engine is not loaded; call initEngine first');
	return JSON.parse(replay(JSON.stringify(format), JSON.stringify(start), JSON.stringify(events)));
}

/** The engine's input for a view: its effective events, without the seqs. */
export function eventsOf(view: MatchView): Event[] {
	return view.events.map((e) => e.event);
}
