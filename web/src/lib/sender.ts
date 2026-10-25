// Sends the fence screen's taps as live commands, one at a time and in tap order (two in flight
// at once could land swapped). A request that never reached the server is resent unchanged,
// same client_id, with backoff: client_id makes the resend idempotent. The offline queue that
// survives a reload comes with Task 10; until then a pending tap lives as long as the page.

import { ApiError, type CommandOutcome } from './api';
import { backoffDelay } from './stream';
import type { Command, CommandRequest, ErrorKind, MatchView, Team } from './types';

export type Intent = { kind: 'point'; team: Team } | { kind: 'undo' };

export interface Pending {
	intent: Intent;
	/** The revision this tap was made on (or last resent on). */
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

export interface SenderState {
	pending: Pending[];
	/** The last attempt could not reach the server; it is being retried. */
	offline: boolean;
}

type Timer = ReturnType<typeof setTimeout>;

export interface SenderDeps {
	post(req: CommandRequest): Promise<CommandOutcome>;
	/** The latest server view the page holds. */
	view(): MatchView | null;
	newClientId(): string;
	setTimeout(f: () => void, ms: number): Timer;
	clearTimeout(t: Timer): void;
	/** A view the server answered with (applied or conflict). */
	onView(view: MatchView): void;
	onDropped(dropped: Dropped): void;
	onState(state: SenderState): void;
}

interface Job {
	intent: Intent;
	base: number;
	/** Fixed at the first send so a resend is the same request. */
	req: CommandRequest | null;
	/** For an undo: the job queued just ahead of it. If that one is dropped, so is this. */
	after: Job | null;
	attempt: number;
	conflicts: number;
}

/** A live point that conflicts is resent fresh this many times before it is given up. */
const MAX_CONFLICTS = 3;

export function createSender(deps: SenderDeps) {
	const queue: Job[] = [];
	let busy = false;
	let offline = false;
	let stopped = false;
	let retry: Timer | null = null;

	const state = (): SenderState => ({
		pending: queue.map((j) => ({ intent: j.intent, base: j.base, sent: j.req !== null })),
		offline
	});
	const emit = () => deps.onState(state());

	function enqueue(intent: Intent, after: Job | null = null) {
		if (stopped) return;
		queue.push({ intent, base: deps.view()?.revision ?? 0, req: null, after, attempt: 0, conflicts: 0 });
		emit();
		void pump();
	}

	/**
	 * Undo the latest point. A tap still waiting to be sent is simply taken back here and never
	 * sent; otherwise the undo waits behind whatever is queued and names the latest event when
	 * its turn comes.
	 */
	function undo() {
		if (stopped) return;
		const tail = queue.at(-1);
		if (tail && tail.intent.kind === 'point' && tail.req === null) {
			queue.pop();
			emit();
			return;
		}
		enqueue({ kind: 'undo' }, tail ?? null);
	}

	function drop(job: Job, reason: DropReason) {
		const i = queue.indexOf(job);
		if (i < 0) return;
		queue.splice(i, 1);
		emit();
		deps.onDropped({ intent: job.intent, reason });
		for (const j of queue.filter((q) => q.after === job)) drop(j, { kind: 'cancelled' });
	}

	function command(intent: Intent): Command | null {
		if (intent.kind === 'point') return { kind: 'point', team: intent.team };
		const last = deps.view()?.events.at(-1);
		return last ? { kind: 'undo', target_seq: last.seq } : null;
	}

	async function pump(): Promise<void> {
		if (busy || stopped || retry !== null) return;
		const job = queue[0];
		if (!job) return;
		busy = true;
		try {
			await send(job);
		} finally {
			busy = false;
		}
		void pump();
	}

	async function send(job: Job) {
		if (!job.req) {
			const cmd = command(job.intent);
			if (!cmd) return drop(job, { kind: 'nothing' });
			job.req = {
				client_id: deps.newClientId(),
				batch: false,
				base_revision: deps.view()?.revision ?? 0,
				commands: [cmd]
			};
		}
		let out: CommandOutcome;
		try {
			out = await deps.post(job.req);
		} catch (e) {
			if (stopped) return;
			if (e instanceof ApiError && e.retryable) {
				offline = true;
				emit();
				retry = deps.setTimeout(() => {
					retry = null;
					void pump();
				}, backoffDelay(job.attempt++));
				return;
			}
			return drop(job, { kind: 'failed', message: e instanceof Error ? e.message : String(e) });
		}
		if (stopped) return;
		job.attempt = 0;
		offline = false;
		switch (out.status) {
			case 'applied': {
				deps.onView(out.view);
				queue.shift();
				// The taps behind this one were made on top of it.
				for (const j of queue) j.base = Math.max(j.base, out.view.revision);
				emit();
				return;
			}
			case 'conflict': {
				deps.onView(out.view);
				// A live point appends whatever the score: send it again as a new command. An undo
				// named an event that is no longer the latest, so it no longer means what was asked.
				if (job.intent.kind === 'point' && job.conflicts++ < MAX_CONFLICTS) {
					job.req = null;
					job.base = out.view.revision;
					emit();
					return;
				}
				return drop(job, { kind: 'conflict' });
			}
			case 'rejected':
				return drop(job, { kind: 'rejected', error: out.kind });
		}
	}

	return {
		point: (team: Team) => enqueue({ kind: 'point', team }),
		undo,
		state,
		/** Stop sending (the page is closing). */
		stop() {
			stopped = true;
			if (retry !== null) deps.clearTimeout(retry);
			retry = null;
		}
	};
}
