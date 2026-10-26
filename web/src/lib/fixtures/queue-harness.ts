// Test helper: a queue wired to scripted server answers, fake timers and an in-memory store.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { vi } from 'vitest';
import { ApiError, type CommandOutcome } from '../api';
import { engineReady, initEngineSync, replayLocal } from '../engine';
import { latestView } from '../fence';
import { createQueue, type Dropped, type QueueState } from '../queue';
import type { CommandRequest, MatchView, ReplaceRequest, Team, ViewEvent } from '../types';
import type { StorageLike } from '../setup';

// The real engine scores the views these tests build and this phone's side of a conflict.
if (!engineReady()) {
	initEngineSync(readFileSync(fileURLToPath(new URL('../wasm/padel_scoring_wasm_bg.wasm', import.meta.url))));
}

export type Answer<R> = (req: R) => Promise<CommandOutcome>;

export class MemoryStorage implements StorageLike {
	readonly map = new Map<string, string>();
	getItem(key: string) {
		return this.map.get(key) ?? null;
	}
	setItem(key: string, value: string) {
		this.map.set(key, value);
	}
	removeItem(key: string) {
		this.map.delete(key);
	}
}

export const TOKEN = 'tok';
export const KEY = `padel-score:queue:${TOKEN}`;

export interface HarnessOptions {
	view?: MatchView | null;
	storage?: MemoryStorage;
	replace?: Answer<ReplaceRequest>[];
	/** Client ids start after this many (a second queue on the same store must not reuse ids). */
	idsFrom?: number;
	/** The queue's clock for the double-tap guard. Default: each reading 1 s after the last, so
	 *  taps in a row are never taken for a double tap. */
	now?: () => number;
	/** Client ids; default `id-1`, `id-2`, ... */
	newClientId?: () => string;
}

export function harness(answers: Answer<CommandRequest>[], opts: HarnessOptions = {}) {
	let view: MatchView | null = opts.view === undefined ? null : opts.view;
	const storage = opts.storage ?? new MemoryStorage();
	const sent: CommandRequest[] = [];
	const replaced: ReplaceRequest[] = [];
	const views: MatchView[] = [];
	const dropped: Dropped[] = [];
	const states: QueueState[] = [];
	const replaceAnswers = opts.replace ?? [];
	let ids = opts.idsFrom ?? 0;
	let clock = 0;
	let viewReads = 0;
	const post = vi.fn(async (_token: string, req: CommandRequest) => {
		sent.push(structuredClone(req));
		const answer = answers.shift();
		if (!answer) throw new Error('no answer queued');
		return answer(req);
	});
	const replace = vi.fn(async (_token: string, req: ReplaceRequest) => {
		replaced.push(structuredClone(req));
		const answer = replaceAnswers.shift();
		if (!answer) throw new Error('no replace answer queued');
		return answer(req);
	});
	const queue = createQueue(TOKEN, storage, { post, replace }, replayLocal, {
		view: () => {
			viewReads++;
			return view;
		},
		newClientId: opts.newClientId ?? (() => `id-${++ids}`),
		now: opts.now ?? (() => (clock += 1000)),
		setTimeout: (f, ms) => setTimeout(f, ms),
		clearTimeout: (t) => clearTimeout(t),
		onView: (v2) => {
			views.push(v2);
			// As the page does: a view older than the one shown is stale.
			view = latestView(view, v2);
		},
		onDropped: (d) => dropped.push(d),
		onState: (s) => states.push(s)
	});
	return {
		queue,
		/** The Task 9 tests' name for it. */
		sender: queue,
		storage,
		sent,
		replaced,
		views,
		dropped,
		states,
		post,
		replace,
		setView: (v2: MatchView) => (view = v2),
		view: () => view,
		/** How many times the queue has asked for the page's view. */
		viewReads: () => viewReads
	};
}

export const applied = (view: MatchView) => async (): Promise<CommandOutcome> => ({ status: 'applied', view });
export const conflict = (view: MatchView) => async (): Promise<CommandOutcome> => ({ status: 'conflict', view });
/** The coordinator typed the result: the match is locked to this phone. */
export const locked = (view: MatchView) => async (): Promise<CommandOutcome> => ({ status: 'locked', view });
export const offline = async (): Promise<CommandOutcome> => {
	throw new ApiError(0, 'network', 'down', true);
};
export const hang = () => new Promise<CommandOutcome>(() => {});
export const tick = (ms = 0) => vi.advanceTimersByTimeAsync(ms);

/**
 * `view` with `teams` points appended as new events (seqs past head_seq) and `revisions` more
 * applied commands, scored by the real engine.
 */
export function grow(view: MatchView, teams: Team[], revisions = teams.length): MatchView {
	let seq = view.head_seq;
	const events: ViewEvent[] = [...view.events, ...teams.map((t) => ({ seq: ++seq, event: { point: t } }))];
	return rescore({ ...view, events, head_seq: seq, revision: view.revision + revisions });
}

/** `view` with its last effective event undone (one more revision; head_seq unchanged). */
export function undone(view: MatchView): MatchView {
	return rescore({ ...view, events: view.events.slice(0, -1), revision: view.revision + 1 });
}

function rescore(view: MatchView): MatchView {
	const out = replayLocal(
		view.format,
		view.start,
		view.events.map((e) => e.event)
	);
	if (!('ok' in out)) throw new Error(JSON.stringify(out));
	return { ...view, score: out.ok };
}

/** The teams of a view's effective points, in order. */
export const points = (view: MatchView | null) =>
	(view?.events ?? []).map((e) => (typeof e.event === 'string' ? e.event : e.event.point));
