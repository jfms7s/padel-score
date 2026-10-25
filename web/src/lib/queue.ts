// The fence screen's taps on their way to the server, and the ones that wait for it.
//
// Online, each tap is a live command, sent one at a time in tap order (two in flight could land
// swapped). A request that never reached the server is resent unchanged, same client_id, with
// backoff: client_id makes the resend idempotent.
//
// Once a send fails on the network (or the stream reports offline) taps are queued instead, on
// top of the last server view seen (the base). On reconnect the whole queue goes as one batch
// that must land on that base revision. If the server moved meanwhile (the watch kept scoring)
// it answers with a conflict, and nothing is merged silently: the page shows both scores and
// someone picks one. "Keep this phone's score" replaces the server's events after the base seq
// with this phone's taps.
//
// Everything here is kept in the store under `padel-score:queue:{token}` after every change, so
// a reload loses nothing: a request already out is resent with its client_id.

import { ApiError, type CommandOutcome } from './api';
import { overlayTaps } from './fence';
import { isEvent } from './guards';
import { backoffDelay } from './stream';
import type {
	Command,
	CommandRequest,
	ErrorKind,
	Event,
	Format,
	MatchView,
	ReplaceRequest,
	ReplayResult,
	Start,
	Team,
	ViewEvent
} from './types';
import type { StorageLike } from './setup';

export type Intent = { kind: 'point'; team: Team } | { kind: 'undo' };

/** A live tap on its way (or about to be). */
export interface Pending {
	intent: Intent;
	/** The revision this tap was made on, or once sent, its request's base_revision. */
	base: number;
	/** It has gone to the server at least once, so a server view may already hold it. */
	sent: boolean;
}

export type DropReason =
	| { kind: 'rejected'; error: ErrorKind }
	| { kind: 'conflict' }
	| { kind: 'nothing' }
	/** An undo queued behind a tap that was dropped: it would undo the wrong point. */
	| { kind: 'cancelled' }
	| { kind: 'failed'; message: string };

export interface Dropped {
	intent: Intent;
	reason: DropReason;
}

/** Why the queued taps can't go as they are. They stay until discarded or changed. */
export type Problem = { kind: 'rejected'; error: ErrorKind } | { kind: 'failed'; message: string };

export interface Conflict {
	/** The score on the server. */
	server: MatchView;
	/** The score on this phone: the server's events up to the base seq, then the queued taps. Null
	 *  when the engine can't score them (not loaded, or it refuses a tap). */
	mine: MatchView | null;
}

export interface QueueState {
	/** Live taps, in order. */
	pending: Pending[];
	/** Taps made offline, in order, waiting for the batch. */
	queued: Intent[];
	/** How many of `queued` (from the front) are in the request on its way. */
	sending: number;
	/** The revision the queued taps were made on. */
	base: number | null;
	/** Taps are being queued rather than sent live. */
	offline: boolean;
	/** The server moved while this phone was offline: someone has to pick a score. */
	conflict: Conflict | null;
	/** "Keep this phone's score" is on its way. */
	resolving: boolean;
	problem: Problem | null;
}

export type FlushOutcome =
	| { status: 'idle' }
	| { status: 'applied' }
	| { status: 'conflict'; conflict: Conflict }
	| { status: 'rejected'; problem: Problem }
	/** It didn't reach the server; it is being retried. */
	| { status: 'offline' };

export interface QueueApi {
	post(token: string, req: CommandRequest): Promise<CommandOutcome>;
	replace(token: string, req: ReplaceRequest): Promise<CommandOutcome>;
}

export type Replay = (format: Format, start: Start, events: Event[]) => ReplayResult;

type Timer = ReturnType<typeof setTimeout>;

export interface QueueHost {
	/** The latest server view the page holds. */
	view(): MatchView | null;
	newClientId(): string;
	setTimeout(f: () => void, ms: number): Timer;
	clearTimeout(t: Timer): void;
	/** A view the server answered with (applied or conflict). */
	onView(view: MatchView): void;
	onDropped(dropped: Dropped): void;
	onState(state: QueueState): void;
}

/** The server takes at most this many commands in one request (crates/server MAX_COMMANDS). */
export const MAX_BATCH = 200;
/** A live point that conflicts is resent fresh this many times before it is given up. */
const MAX_CONFLICTS = 3;

export const queueKey = (token: string) => `padel-score:queue:${token}`;

// ---- pure helpers ------------------------------------------------------------------------

interface Simulated {
	events: ViewEvent[];
	commands: Command[];
	/** Undos that found nothing left to undo. */
	skipped: number;
}

/**
 * Play `intents` over `events` the way the server would: a point appends with the next seq past
 * `headSeq` (seqs are never reused), an undo takes back the latest event, whatever made it. The
 * commands are what to send for that: an undo names the seq it takes back.
 */
export function simulate(events: ViewEvent[], headSeq: number, intents: Intent[]): Simulated {
	const out = [...events];
	const commands: Command[] = [];
	let next = headSeq;
	let skipped = 0;
	for (const intent of intents) {
		if (intent.kind === 'point') {
			out.push({ seq: ++next, event: { point: intent.team } });
			commands.push({ kind: 'point', team: intent.team });
			continue;
		}
		const last = out.pop();
		if (last) commands.push({ kind: 'undo', target_seq: last.seq });
		else skipped++;
	}
	return { events: out, commands, skipped };
}

function scored(view: MatchView, events: ViewEvent[], replay: Replay): MatchView | null {
	try {
		const out = replay(
			view.format,
			view.start,
			events.map((e) => e.event)
		);
		return 'ok' in out ? { ...view, events, score: out.ok } : null;
	} catch {
		// The engine isn't loaded.
		return null;
	}
}

/**
 * What to show: the server's view with the taps not in it yet laid over. Live taps as Task 9
 * decided (see `overlayTaps`); the queued ones replayed on top. The taps of a batch on its way
 * are left out once a view newer than their base is in, since it holds them (or the conflict
 * choice is coming), and all of them while that choice is open. Falls back to the server's view when the engine can't score.
 */
export function localView(server: MatchView, state: QueueState, replay: Replay): MatchView {
	const live = overlayTaps(server, state.pending).map((team): Intent => ({ kind: 'point', team }));
	const held = state.base !== null && server.revision > state.base ? state.sending : 0;
	// While the choice is open the screen shows the server's score; the sheet shows both.
	const queued = state.conflict ? [] : state.queued.slice(held);
	const intents = [...live, ...queued];
	if (intents.length === 0) return server;
	const sim = simulate(server.events, server.head_seq, intents);
	return scored(server, sim.events, replay) ?? server;
}

/** The score on this phone in a conflict: the server's events up to `baseSeq`, then `intents`. */
export function mineView(server: MatchView, baseSeq: number, intents: Intent[], replay: Replay): MatchView | null {
	const kept = server.events.filter((e) => e.seq <= baseSeq);
	return scored(server, simulate(kept, server.head_seq, intents).events, replay);
}

// ---- stored shape ------------------------------------------------------------------------

interface Base {
	revision: number;
	head_seq: number;
	events: ViewEvent[];
}

interface StoredJob {
	id: number;
	intent: Intent;
	base: number;
	req: CommandRequest | null;
	after: number | null;
	target: number | 'none' | null;
	conflicts: number;
}

interface Outstanding {
	kind: 'batch' | 'replace';
	/** How many of the queued taps (from the front) it carries. */
	count: number;
	req: CommandRequest | ReplaceRequest;
}

interface QueuedItem {
	intent: Intent;
	/** For an undo made while a live tap was still on its way: that tap's job id. */
	after: number | null;
}

interface Stored {
	v: 1;
	next: number;
	jobs: StoredJob[];
	queued: QueuedItem[];
	/** Live taps that landed on top of someone else's: on the server already, but part of mine. */
	carried: Intent[];
	base: Base | null;
	outstanding: Outstanding | null;
}

type Check = (v: unknown) => boolean;
const obj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);
const int: Check = (v) => Number.isInteger(v) && (v as number) >= 0;
const str: Check = (v) => typeof v === 'string';
const arr = (c: Check): Check => (v) => Array.isArray(v) && v.every(c);
const nul = (c: Check): Check => (v) => v === null || c(v);
const team: Check = (v) => v === 'a' || v === 'b';
const intent: Check = (v) =>
	obj(v) && ((v.kind === 'point' && team(v.team)) || (v.kind === 'undo' && Object.keys(v).length === 1));
const command: Check = (v) =>
	obj(v) &&
	((v.kind === 'point' && team(v.team)) ||
		v.kind === 'time_up' ||
		(v.kind === 'undo' && int(v.target_seq)));
const request: Check = (v) =>
	obj(v) &&
	str(v.client_id) &&
	int(v.base_revision) &&
	arr(command)(v.commands) &&
	(typeof v.batch === 'boolean' || int(v.base_seq));
const baseShape: Check = (v) =>
	obj(v) && int(v.revision) && int(v.head_seq) && arr((e) => obj(e) && int(e.seq) && isEvent(e.event))(v.events);
const job: Check = (v) =>
	obj(v) &&
	int(v.id) &&
	intent(v.intent) &&
	int(v.base) &&
	nul(request)(v.req) &&
	nul(int)(v.after) &&
	(v.target === null || v.target === 'none' || int(v.target)) &&
	int(v.conflicts);
const stored: Check = (v) =>
	obj(v) &&
	v.v === 1 &&
	int(v.next) &&
	arr(job)(v.jobs) &&
	arr((q) => obj(q) && intent(q.intent) && nul(int)(q.after))(v.queued) &&
	arr(intent)(v.carried) &&
	nul(baseShape)(v.base) &&
	nul((o) => obj(o) && (o.kind === 'batch' || o.kind === 'replace') && int(o.count) && request(o.req))(v.outstanding);

type Loaded = { kind: 'none' } | { kind: 'ok'; value: Stored } | { kind: 'discarded' };

function load(storage: StorageLike, key: string): Loaded {
	let raw: string | null;
	try {
		raw = storage.getItem(key);
	} catch {
		return { kind: 'none' };
	}
	if (raw === null) return { kind: 'none' };
	try {
		const value: unknown = JSON.parse(raw);
		if (stored(value)) return { kind: 'ok', value: value as Stored };
	} catch {
		// Unreadable: discarded below.
	}
	return { kind: 'discarded' };
}

// ---- the queue ---------------------------------------------------------------------------

interface Job {
	id: number;
	intent: Intent;
	base: number;
	/** Fixed at the first send so a resend is the same request. */
	req: CommandRequest | null;
	/** For an undo: the job queued just ahead of it. If that one is dropped, so is this. */
	after: Job | null;
	/** For an undo behind a job that landed: the seq that job's answer ended on ('none': nothing). */
	target: number | 'none' | null;
	attempt: number;
	conflicts: number;
}

type Mode = 'live' | 'offline' | 'conflict' | 'rejected';

export function createQueue(token: string, storage: StorageLike, api: QueueApi, replay: Replay, host: QueueHost) {
	const key = queueKey(token);
	let jobs: Job[] = [];
	let queued: QueuedItem[] = [];
	let carried: Intent[] = [];
	let base: Base | null = null;
	let outstanding: Outstanding | null = null;
	let nextId = 1;
	let mode: Mode = 'live';
	let conflictView: MatchView | null = null;
	let problem: Problem | null = null;
	/** Whether to try the batch: the line is believed up. */
	let online = true;
	let busy = false;
	let stopped = false;
	let retry: Timer | null = null;
	let attempt = 0;
	let waiters: ((o: FlushOutcome) => void)[] = [];

	const loaded = load(storage, key);
	if (loaded.kind === 'ok') {
		const s = loaded.value;
		const byId = new Map<number, Job>();
		jobs = s.jobs.map((j) => {
			const restored: Job = { ...j, after: j.after === null ? null : (byId.get(j.after) ?? null), attempt: 0 };
			byId.set(j.id, restored);
			return restored;
		});
		queued = s.queued;
		carried = s.carried;
		base = s.base;
		outstanding = s.outstanding;
		nextId = Math.max(s.next, ...s.jobs.map((j) => j.id + 1));
		if (queued.length > 0 || outstanding !== null) mode = 'offline';
	} else if (loaded.kind === 'discarded') {
		forget();
	}

	function forget() {
		try {
			if (storage.removeItem) storage.removeItem(key);
			else storage.setItem(key, 'null');
		} catch {
			// Nothing to keep anyway.
		}
	}

	function persist() {
		if (jobs.length === 0 && queued.length === 0 && outstanding === null) return forget();
		const s: Stored = {
			v: 1,
			next: nextId,
			jobs: jobs.map((j) => ({
				id: j.id,
				intent: j.intent,
				base: j.base,
				req: j.req,
				after: j.after?.id ?? null,
				target: j.target,
				conflicts: j.conflicts
			})),
			queued,
			carried,
			base,
			outstanding
		};
		try {
			storage.setItem(key, JSON.stringify(s));
		} catch {
			// A full or blocked store: the queue still lives as long as the page.
		}
	}

	function conflictOf(): Conflict | null {
		if (mode !== 'conflict' || !conflictView) return null;
		const baseSeq = base?.head_seq ?? conflictView.head_seq;
		return { server: conflictView, mine: mineView(conflictView, baseSeq, mine(), replay) };
	}

	const mine = (): Intent[] => [...carried, ...queued.map((q) => q.intent)];

	const state = (): QueueState => ({
		pending: jobs.map((j) => ({ intent: j.intent, base: j.base, sent: j.req !== null })),
		queued: queued.map((q) => q.intent),
		sending: outstanding?.count ?? 0,
		base: base?.revision ?? null,
		offline: mode !== 'live',
		conflict: conflictOf(),
		resolving: outstanding?.kind === 'replace',
		problem
	});

	/** After every change: keep it, tell the page. */
	function changed() {
		persist();
		host.onState(state());
	}

	function resolve(o: FlushOutcome) {
		const w = waiters;
		waiters = [];
		for (const f of w) f(o);
	}

	function snapshot(view: MatchView): Base {
		return { revision: view.revision, head_seq: view.head_seq, events: view.events };
	}

	/** Back to live sending once nothing waits for a batch and the line is up. */
	function settle() {
		if (mode !== 'offline' || !online || outstanding !== null) return;
		if (queued.length > 0) return;
		mode = 'live';
		base = null;
		carried = [];
	}

	/** The queue changed by a tap: a stuck queue gets another go. */
	function retake() {
		if (mode === 'rejected') {
			mode = 'offline';
			problem = null;
		}
	}

	// ---- taps ----

	function enqueueLive(intent: Intent, after: Job | null = null) {
		jobs.push({
			id: nextId++,
			intent,
			base: host.view()?.revision ?? 0,
			req: null,
			after,
			target: null,
			attempt: 0,
			conflicts: 0
		});
		changed();
		void pump();
	}

	function queue(item: QueuedItem): boolean {
		if (!base) {
			const view = host.view();
			if (!view) return false;
			base = snapshot(view);
		}
		queued.push(item);
		retake();
		changed();
		void pump();
		return true;
	}

	function point(team: Team) {
		if (stopped) return;
		const intent: Intent = { kind: 'point', team };
		if (mode === 'live' || !queue({ intent, after: null })) enqueueLive(intent);
	}

	function undo() {
		if (stopped) return;
		if (mode === 'live') return undoLive();
		const tail = queued.at(-1);
		if (tail && tail.intent.kind === 'point' && queued.length > (outstanding?.count ?? 0)) {
			queued.pop();
			retake();
			settle();
			changed();
			void pump();
			return;
		}
		if (queued.length === 0 && jobs.length > 0) {
			const last = jobs.at(-1);
			if (last && last.intent.kind === 'point' && last.req === null) {
				jobs.pop();
				changed();
				return;
			}
			// It undoes the live tap still on its way, once that one lands.
			if (!queue({ intent: { kind: 'undo' }, after: last?.id ?? null })) undoLive();
			return;
		}
		const on = base ?? (host.view() ? snapshot(host.view() as MatchView) : null);
		const all = [...queued.map((q) => q.intent), { kind: 'undo' } as Intent];
		if (!on || simulate(on.events, on.head_seq, all).skipped > 0) {
			host.onDropped({ intent: { kind: 'undo' }, reason: { kind: 'nothing' } });
			return;
		}
		queue({ intent: { kind: 'undo' }, after: null });
	}

	/**
	 * Online undo (Task 9): a tap still waiting to be sent is simply taken back here and never
	 * sent; otherwise the undo waits behind whatever is queued and names the latest event when
	 * its turn comes.
	 */
	function undoLive() {
		const tail = jobs.at(-1);
		if (tail && tail.intent.kind === 'point' && tail.req === null) {
			jobs.pop();
			changed();
			return;
		}
		enqueueLive({ kind: 'undo' }, tail ?? null);
	}

	function drop(job: Job, reason: DropReason) {
		const i = jobs.indexOf(job);
		if (i < 0) return;
		jobs.splice(i, 1);
		host.onDropped({ intent: job.intent, reason });
		for (const j of jobs.filter((q) => q.after === job)) drop(j, { kind: 'cancelled' });
		const bound = queued.filter((q) => q.after === job.id);
		if (bound.length > 0) {
			queued = queued.filter((q) => q.after !== job.id);
			for (const q of bound) host.onDropped({ intent: q.intent, reason: { kind: 'cancelled' } });
			if (queued.length === 0 && outstanding === null) {
				base = null;
				carried = [];
			}
		}
		settle();
		changed();
	}

	// ---- sending ----

	function scheduleRetry() {
		retry = host.setTimeout(() => {
			retry = null;
			online = true;
			void pump();
		}, backoffDelay(attempt++));
	}

	async function pump(): Promise<void> {
		if (busy || stopped || retry !== null) return;
		busy = true;
		try {
			const job = jobs[0];
			if (job) await sendLive(job);
			else if (outstanding?.kind === 'replace') await sendOutstanding();
			else if (mode === 'offline' && online && (outstanding !== null || (base !== null && queued.length > 0))) {
				buildBatch();
				await sendOutstanding();
			} else return;
		} finally {
			busy = false;
		}
		void pump();
	}

	function command(job: Job): Command | null {
		if (job.intent.kind === 'point') return { kind: 'point', team: job.intent.team };
		if (job.target === 'none') return null;
		if (job.target !== null) return { kind: 'undo', target_seq: job.target };
		const last = host.view()?.events.at(-1);
		return last ? { kind: 'undo', target_seq: last.seq } : null;
	}

	async function sendLive(job: Job) {
		if (!job.req) {
			const cmd = command(job);
			if (!cmd) return drop(job, { kind: 'nothing' });
			job.req = {
				client_id: host.newClientId(),
				batch: false,
				base_revision: host.view()?.revision ?? 0,
				commands: [cmd]
			};
			// Once out, the tap shows until a view newer than the one it was sent on.
			job.base = job.req.base_revision;
			changed();
		}
		let out: CommandOutcome;
		try {
			out = await api.post(token, job.req);
		} catch (e) {
			if (stopped) return;
			if (e instanceof ApiError && e.retryable) {
				online = false;
				if (mode === 'live') mode = 'offline';
				changed();
				attempt = job.attempt;
				scheduleRetry();
				job.attempt = attempt;
				return;
			}
			return drop(job, { kind: 'failed', message: e instanceof Error ? e.message : String(e) });
		}
		if (stopped) return;
		job.attempt = 0;
		attempt = 0;
		online = true;
		switch (out.status) {
			case 'applied': {
				const view = out.view;
				host.onView(view);
				jobs.shift();
				// The taps behind this one were made on top of it.
				for (const j of jobs) j.base = Math.max(j.base, view.revision);
				// An undo bound behind it undoes what it made: its answer's latest event.
				const landed = view.events.at(-1)?.seq ?? 'none';
				for (const j of jobs) if (j.after === job) j.target = landed;
				if (base && queued.length > 0) adopt(job, view);
				settle();
				changed();
				return;
			}
			case 'conflict': {
				host.onView(out.view);
				// A live point appends whatever the score: send it again as a new command. An undo
				// named an event that is no longer the latest, so it no longer means what was asked.
				if (job.intent.kind === 'point' && job.conflicts++ < MAX_CONFLICTS) {
					job.req = null;
					job.base = out.view.revision;
					changed();
					return;
				}
				return drop(job, { kind: 'conflict' });
			}
			case 'rejected':
				return drop(job, { kind: 'rejected', error: out.kind });
		}
	}

	/**
	 * A live tap landed while taps queue behind it. If it landed straight on the base, the batch
	 * builds on its answer. If someone else's commands got in first, the batch stays on the old
	 * base (so it can't merge silently) and the tap is carried into "mine" for the choice.
	 */
	function adopt(job: Job, view: MatchView) {
		if (!base || view.revision <= base.revision) return;
		if (view.revision === base.revision + 1) base = snapshot(view);
		else carried.push(job.intent);
	}

	function buildBatch() {
		if (outstanding || !base) return;
		const count = Math.min(queued.length, MAX_BATCH);
		const sim = simulate(
			base.events,
			base.head_seq,
			queued.slice(0, count).map((q) => q.intent)
		);
		if (sim.commands.length === 0) {
			queued.splice(0, count);
			settle();
			changed();
			return;
		}
		outstanding = {
			kind: 'batch',
			count,
			req: { client_id: host.newClientId(), batch: true, base_revision: base.revision, commands: sim.commands }
		};
		changed();
	}

	async function sendOutstanding() {
		const o = outstanding;
		if (!o) return;
		let out: CommandOutcome;
		try {
			out = o.kind === 'batch' ? await api.post(token, o.req as CommandRequest) : await api.replace(token, o.req as ReplaceRequest);
		} catch (e) {
			if (stopped) return;
			if (e instanceof ApiError && e.retryable) {
				online = false;
				changed();
				scheduleRetry();
				resolve({ status: 'offline' });
				return;
			}
			return stuck({ kind: 'failed', message: e instanceof Error ? e.message : String(e) });
		}
		if (stopped) return;
		attempt = 0;
		online = true;
		switch (out.status) {
			case 'applied': {
				host.onView(out.view);
				queued.splice(0, o.count);
				outstanding = null;
				carried = [];
				conflictView = null;
				base = queued.length > 0 ? snapshot(out.view) : null;
				mode = 'offline';
				settle();
				changed();
				resolve({ status: 'applied' });
				return;
			}
			case 'conflict': {
				host.onView(out.view);
				// Never saved by the server: the next try gets a fresh client_id.
				outstanding = null;
				conflictView = out.view;
				mode = 'conflict';
				changed();
				const c = conflictOf();
				if (c) resolve({ status: 'conflict', conflict: c });
				return;
			}
			case 'rejected':
				return stuck({ kind: 'rejected', error: out.kind });
		}
	}

	/** The server won't take the queue as it stands. It stays, for the scorer to change or drop. */
	function stuck(p: Problem) {
		outstanding = null;
		conflictView = null;
		problem = p;
		mode = 'rejected';
		changed();
		resolve({ status: 'rejected', problem: p });
	}

	function wait(): Promise<FlushOutcome> {
		return new Promise((r) => waiters.push(r));
	}

	/** The line is (maybe) back: try now rather than after the backoff. */
	function reconnect() {
		if (stopped) return;
		online = true;
		if (retry !== null) {
			host.clearTimeout(retry);
			retry = null;
		}
		settle();
		changed();
		void pump();
	}

	function clear() {
		queued = [];
		carried = [];
		base = null;
		outstanding = null;
		conflictView = null;
		problem = null;
		mode = online ? 'live' : 'offline';
		changed();
		void pump();
	}

	// Taps left from before a reload go now.
	if (loaded.kind === 'ok') void pump();

	return {
		point,
		undo,
		state,
		/** Whether a stored queue couldn't be read and was dropped (say so). */
		discardedOnLoad: loaded.kind === 'discarded',
		/** The stream's view of the line. */
		connection(status: 'live' | 'offline') {
			if (status === 'live') return reconnect();
			if (stopped) return;
			online = false;
			if (mode === 'live') mode = 'offline';
			changed();
		},
		reconnect,
		/** Send the queue now; resolves with what came of it. */
		flush(): Promise<FlushOutcome> {
			if (mode === 'conflict') {
				const c = conflictOf();
				return Promise.resolve(c ? { status: 'conflict', conflict: c } : { status: 'idle' });
			}
			if (mode === 'rejected' && problem) return Promise.resolve({ status: 'rejected', problem });
			if (queued.length === 0 && outstanding === null) return Promise.resolve({ status: 'idle' });
			const done = wait();
			reconnect();
			return done;
		},
		/** The screen with the taps not on the server yet. */
		localView: (server: MatchView) => localView(server, state(), replay),
		/** Conflict: drop this phone's taps, keep the server's score. */
		keepServer() {
			if (mode !== 'conflict' || outstanding) return;
			clear();
		},
		/** Conflict: put this phone's taps in place of the server's after the base seq. */
		keepMine(): Promise<FlushOutcome> {
			if (mode !== 'conflict' || !conflictView || outstanding) return Promise.resolve({ status: 'idle' });
			const baseSeq = base?.head_seq ?? conflictView.head_seq;
			const kept = conflictView.events.filter((e) => e.seq <= baseSeq);
			const sim = simulate(kept, conflictView.head_seq, mine());
			outstanding = {
				kind: 'replace',
				count: queued.length,
				req: {
					client_id: host.newClientId(),
					base_revision: conflictView.revision,
					base_seq: baseSeq,
					commands: sim.commands
				}
			};
			changed();
			const done = wait();
			void pump();
			return done;
		},
		/** A stuck queue: drop the taps. */
		discard() {
			if (mode !== 'rejected') return;
			clear();
		},
		/** Stop sending (the page is closing). What is queued stays in the store. */
		stop() {
			stopped = true;
			if (retry !== null) host.clearTimeout(retry);
			retry = null;
		}
	};
}

export type Queue = ReturnType<typeof createQueue>;
