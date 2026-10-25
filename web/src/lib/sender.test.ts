import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { overlayTaps } from './fence';
import { createSender, type Dropped, type SenderState } from './sender';
import { ApiError, postCommands, REQUEST_TIMEOUT_MS, type CommandOutcome } from './api';
import type { CommandRequest, MatchView } from './types';
import { asView } from './fixtures/load';
import onePoint from './fixtures/view-one-point.json';
import newMatch from './fixtures/view-new.json';

const start = asView(newMatch);
const after = asView(onePoint);

function harness(answers: ((req: CommandRequest) => Promise<CommandOutcome>)[]) {
	let view: MatchView | null = start;
	const sent: CommandRequest[] = [];
	const views: MatchView[] = [];
	const dropped: Dropped[] = [];
	const states: SenderState[] = [];
	let ids = 0;
	const post = vi.fn(async (req: CommandRequest) => {
		sent.push(structuredClone(req));
		const answer = answers.shift();
		if (!answer) throw new Error('no answer queued');
		return answer(req);
	});
	const sender = createSender({
		post,
		view: () => view,
		newClientId: () => `id-${++ids}`,
		setTimeout: (f, ms) => setTimeout(f, ms),
		clearTimeout: (t) => clearTimeout(t),
		onView: (v2) => {
			views.push(v2);
			view = v2;
		},
		onDropped: (d) => dropped.push(d),
		onState: (s) => states.push(s)
	});
	return { sender, sent, views, dropped, states, post, setView: (v2: MatchView) => (view = v2) };
}

const applied = (view: MatchView) => async (): Promise<CommandOutcome> => ({ status: 'applied', view });
const offline = async (): Promise<CommandOutcome> => {
	throw new ApiError(0, 'network', 'down', true);
};
const tick = (ms = 0) => vi.advanceTimersByTimeAsync(ms);

beforeEach(() => {
	vi.useFakeTimers();
});
afterEach(() => {
	vi.useRealTimers();
});

describe('createSender', () => {
	it('sends a live point and reports the applied view', async () => {
		const h = harness([applied(after)]);
		h.sender.point('a');
		expect(h.sender.state().pending.map((p) => p.intent)).toEqual([{ kind: 'point', team: 'a' }]);
		expect(h.sender.state().pending[0]?.base).toBe(start.revision);
		await tick();
		expect(h.sent).toEqual([
			{ client_id: 'id-1', batch: false, base_revision: start.revision, commands: [{ kind: 'point', team: 'a' }] }
		]);
		expect(h.views).toEqual([after]);
		expect(h.sender.state().pending).toEqual([]);
	});

	it('sends taps one at a time, in order, rebasing the rest on each applied view', async () => {
		const second = { ...after, revision: after.revision + 1 };
		let release!: () => void;
		const gate = new Promise<void>((r) => (release = r));
		const h = harness([
			async () => {
				await gate;
				return { status: 'applied', view: after };
			},
			applied(second)
		]);
		h.sender.point('a');
		h.sender.point('b');
		await tick();
		expect(h.post).toHaveBeenCalledTimes(1);
		release();
		await tick();
		expect(h.sent.map((r) => r.commands)).toEqual([[{ kind: 'point', team: 'a' }], [{ kind: 'point', team: 'b' }]]);
		expect(h.sent[1]?.base_revision).toBe(after.revision);
		// While the second was in flight it was rebased on the first's view.
		expect(h.states.some((s) => s.pending.length === 1 && s.pending[0]?.base === after.revision)).toBe(true);
	});

	it('undo targets the latest event at send time', async () => {
		const h = harness([applied(after), applied(start)]);
		h.sender.point('a');
		h.sender.undo();
		await tick();
		const last = after.events.at(-1)?.seq;
		expect(h.sent[1]?.commands).toEqual([{ kind: 'undo', target_seq: last }]);
	});

	it('drops an undo with nothing to undo', async () => {
		const h = harness([]);
		h.sender.undo();
		await tick();
		expect(h.post).not.toHaveBeenCalled();
		expect(h.dropped).toEqual([{ intent: { kind: 'undo' }, reason: { kind: 'nothing' } }]);
	});

	it('drops a rejected tap', async () => {
		const h = harness([async () => ({ status: 'rejected', kind: 'match_over', index: 0 })]);
		h.sender.point('a');
		await tick();
		expect(h.dropped).toEqual([{ intent: { kind: 'point', team: 'a' }, reason: { kind: 'rejected', error: 'match_over' } }]);
		expect(h.sender.state().pending).toEqual([]);
	});

	it('adopts the view on a conflict and resends a point fresh', async () => {
		const newer = { ...after, revision: 9 };
		const h = harness([async () => ({ status: 'conflict', view: newer }), applied({ ...newer, revision: 10 })]);
		h.sender.point('b');
		await tick();
		expect(h.views[0]).toBe(newer);
		expect(h.sent.map((r) => [r.client_id, r.base_revision])).toEqual([
			['id-1', start.revision],
			['id-2', 9]
		]);
		expect(h.dropped).toEqual([]);
	});

	it('drops an undo that conflicts', async () => {
		const h = harness([async () => ({ status: 'conflict', view: after })]);
		h.setView(after);
		h.sender.undo();
		await tick();
		expect(h.views).toEqual([after]);
		expect(h.dropped).toEqual([{ intent: { kind: 'undo' }, reason: { kind: 'conflict' } }]);
	});

	it('retries a network failure with the same request and backoff', async () => {
		const h = harness([offline, offline, offline, applied(after)]);
		h.sender.point('a');
		await tick();
		expect(h.sender.state().offline).toBe(true);
		expect(h.post).toHaveBeenCalledTimes(1);
		await tick(999);
		expect(h.post).toHaveBeenCalledTimes(1);
		await tick(1);
		expect(h.post).toHaveBeenCalledTimes(2);
		await tick(2000);
		expect(h.post).toHaveBeenCalledTimes(3);
		await tick(5000);
		expect(h.post).toHaveBeenCalledTimes(4);
		expect(new Set(h.sent.map((r) => JSON.stringify(r))).size).toBe(1);
		expect(h.sender.state()).toEqual({ pending: [], offline: false });
	});

	it('keeps an undo target fixed across retries', async () => {
		const h = harness([offline, applied(start)]);
		h.setView(after);
		h.sender.undo();
		await tick();
		h.setView({ ...after, events: [], revision: 99 });
		await tick(1000);
		expect(h.sent[0]).toEqual(h.sent[1]);
	});

	it('drops a tap the server refuses outright', async () => {
		const h = harness([
			async () => {
				throw new ApiError(400, 'invalid', 'bad');
			}
		]);
		h.sender.point('a');
		await tick();
		expect(h.dropped).toEqual([{ intent: { kind: 'point', team: 'a' }, reason: { kind: 'failed', message: 'bad' } }]);
	});

	it('stop cancels a pending retry', async () => {
		const h = harness([offline]);
		h.sender.point('a');
		await tick();
		h.sender.stop();
		await tick(60_000);
		expect(h.post).toHaveBeenCalledTimes(1);
	});

	it('marks the head as sent and the rest as not', async () => {
		const gate = new Promise<CommandOutcome>(() => {});
		const h = harness([() => gate]);
		h.sender.point('a');
		h.sender.point('b');
		await tick();
		expect(h.sender.state().pending.map((p) => p.sent)).toEqual([true, false]);
	});

	it('an SSE echo before the HTTP answer keeps tap B visible', async () => {
		const h = harness([() => new Promise<CommandOutcome>(() => {})]);
		h.sender.point('a');
		h.sender.point('b');
		await tick();
		const pending = h.sender.state().pending;
		// Before any echo both show; the echo of A (one revision on) holds A, so only B is laid over.
		expect(overlayTaps(start, pending)).toEqual(['a', 'b']);
		expect(overlayTaps({ ...after, revision: start.revision + 1 }, pending)).toEqual(['b']);
	});

	it('a conflict reset re-overlays the point', async () => {
		let release!: (o: CommandOutcome) => void;
		const h = harness([
			async () => ({ status: 'conflict', view: { ...after, revision: 9 } }),
			() => new Promise<CommandOutcome>((r) => (release = r))
		]);
		h.sender.point('a');
		await tick();
		// Resent fresh on revision 9: in flight again, made on 9, so it shows over 9.
		const pending = h.sender.state().pending;
		expect(pending).toEqual([{ intent: { kind: 'point', team: 'a' }, base: 9, sent: true }]);
		expect(overlayTaps({ ...after, revision: 9 }, pending)).toEqual(['a']);
		release({ status: 'applied', view: { ...after, revision: 10 } });
	});

	it('a post that never resolves is retried with the same client_id', async () => {
		let calls = 0;
		vi.stubGlobal('fetch', (_url: string, init?: RequestInit) => {
			calls++;
			if (calls === 1) {
				return new Promise((_, reject) => {
					init?.signal?.addEventListener('abort', () => reject(new DOMException('timed out', 'TimeoutError')));
				});
			}
			return Promise.resolve(new Response(JSON.stringify(after), { status: 200 }));
		});
		try {
			const sent: CommandRequest[] = [];
			let view: MatchView = start;
			const sender = createSender({
				post: (req) => {
					sent.push(structuredClone(req));
					return postCommands('t', req);
				},
				view: () => view,
				newClientId: () => crypto.randomUUID(),
				setTimeout: (f, ms) => setTimeout(f, ms),
				clearTimeout: (t) => clearTimeout(t),
				onView: (v2) => (view = v2),
				onDropped: () => {},
				onState: () => {}
			});
			sender.point('a');
			await tick(REQUEST_TIMEOUT_MS);
			expect(sender.state().offline).toBe(true);
			await tick(1000);
			expect(sent).toHaveLength(2);
			expect(sent[1]).toEqual(sent[0]);
			expect(view).toEqual(after);
			expect(sender.state().pending).toEqual([]);
		} finally {
			vi.unstubAllGlobals();
		}
	});
});

describe('undo binding', () => {
	const hang = () => new Promise<CommandOutcome>(() => {});

	it('undo cancels a tap that has not been sent, and is never sent itself', async () => {
		const h = harness([hang]);
		h.sender.point('a');
		h.sender.point('b');
		h.sender.undo();
		await tick();
		expect(h.sender.state().pending.map((p) => p.intent)).toEqual([{ kind: 'point', team: 'a' }]);
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.dropped).toEqual([]);
	});

	it('undo behind the tap in flight waits for it, then undoes it', async () => {
		let release!: (o: CommandOutcome) => void;
		const h = harness([() => new Promise<CommandOutcome>((r) => (release = r)), applied(start)]);
		h.sender.point('a');
		h.sender.undo();
		await tick();
		expect(h.post).toHaveBeenCalledTimes(1);
		release({ status: 'applied', view: after });
		await tick();
		expect(h.sent[1]?.commands).toEqual([{ kind: 'undo', target_seq: after.events.at(-1)?.seq }]);
	});

	it('drops the undos queued behind a point that is dropped', async () => {
		let release!: (o: CommandOutcome) => void;
		const h = harness([() => new Promise<CommandOutcome>((r) => (release = r))]);
		h.setView(after);
		h.sender.point('a');
		h.sender.undo();
		h.sender.undo();
		await tick();
		release({ status: 'rejected', kind: 'match_over', index: 0 });
		await tick();
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.dropped.map((d) => d.reason.kind)).toEqual(['rejected', 'cancelled', 'cancelled']);
		expect(h.sender.state().pending).toEqual([]);
	});

	it('drops them too when the point fails outright or its conflicts run out', async () => {
		const h = harness([
			async () => {
				throw new ApiError(400, 'invalid', 'bad');
			}
		]);
		h.sender.point('a');
		h.sender.undo();
		await tick();
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.dropped.map((d) => d.reason.kind)).toEqual(['failed', 'cancelled']);

		const conflict = async (): Promise<CommandOutcome> => ({ status: 'conflict', view: after });
		const c = harness([conflict, conflict, conflict, conflict]);
		c.sender.point('a');
		c.sender.undo();
		await tick();
		expect(c.post).toHaveBeenCalledTimes(4);
		expect(c.dropped.map((d) => d.reason.kind)).toEqual(['conflict', 'cancelled']);
	});

	it('keeps an undo queued behind one that is sent when that one lands', async () => {
		const h = harness([applied(start), applied({ ...start, revision: 50 })]);
		h.setView(after);
		h.sender.undo();
		h.sender.undo();
		await tick();
		// The second undo found nothing left to undo once the first landed (start has no events).
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.dropped.map((d) => d.reason.kind)).toEqual(['nothing']);
	});
});
