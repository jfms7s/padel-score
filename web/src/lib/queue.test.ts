import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, type CommandOutcome } from './api';
import { replayLocal } from './engine';
import { localView, MAX_COMMANDS, mineView, type QueueState } from './queue';
import { asView } from './fixtures/load';
import {
	applied,
	conflict,
	grow,
	hang,
	harness,
	KEY,
	locked,
	MemoryStorage,
	offline,
	points,
	tick,
	undone
} from './fixtures/queue-harness';
import onePoint from './fixtures/view-one-point.json';
import newMatch from './fixtures/view-new.json';
import finished from './fixtures/view-finished.json';

// The offline queue: taps made without the server are kept (on the device), sent as one batch
// on reconnect, and a batch the server can't take as it stands opens an explicit choice.

const seen = asView(onePoint); // revision 1, head_seq 1: the last view before going offline
const stored = (s: MemoryStorage) => JSON.parse(s.getItem(KEY) ?? 'null');

beforeEach(() => {
	vi.useFakeTimers();
});
afterEach(() => {
	vi.useRealTimers();
});

/** A queue that has gone offline on `seen` and taken `teams` as taps. */
function offlineWith(teams: ('a' | 'b')[], answers: Parameters<typeof harness>[0] = [], opts = {}) {
	const h = harness(answers, { view: seen, ...opts });
	h.queue.connection('offline');
	for (const t of teams) h.queue.point(t);
	return h;
}

describe('offline queue', () => {
	it('offline taps queue with base revision and seq', async () => {
		const h = offlineWith(['a', 'b', 'a']);
		await tick();
		expect(h.post).not.toHaveBeenCalled();
		expect(h.queue.state()).toMatchObject({
			offline: true,
			queued: [
				{ kind: 'point', team: 'a' },
				{ kind: 'point', team: 'b' },
				{ kind: 'point', team: 'a' }
			]
		});
		const s = stored(h.storage);
		expect(s).toMatchObject({ v: 1, base: { revision: seen.revision, head_seq: seen.head_seq } });
		expect(s.queued).toHaveLength(3);
		// The score on screen is the server's plus the queued taps.
		expect(points(h.queue.localView(seen))).toEqual(['a', 'a', 'b', 'a']);
	});

	it('a send that fails on the network turns later taps into queued ones', async () => {
		const h = harness([offline], { view: seen });
		h.queue.point('a');
		await tick();
		expect(h.queue.state().offline).toBe(true);
		h.queue.point('b');
		await tick();
		// Tap A is still the live command it was sent as; B waits for the batch.
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.queue.state().pending.map((p) => p.intent)).toEqual([{ kind: 'point', team: 'a' }]);
		expect(h.queue.state().queued).toEqual([{ kind: 'point', team: 'b' }]);
		expect(stored(h.storage).base).toMatchObject({ revision: seen.revision, head_seq: seen.head_seq });
	});

	it('flush applies and clears', async () => {
		const done = grow(seen, ['a', 'b'], 1);
		const h = offlineWith(['a', 'b'], [applied(done)]);
		h.queue.connection('live');
		await tick();
		expect(h.sent).toEqual([
			{
				client_id: 'id-1',
				batch: true,
				base_revision: seen.revision,
				commands: [
					{ kind: 'point', team: 'a' },
					{ kind: 'point', team: 'b' }
				]
			}
		]);
		expect(h.views).toEqual([done]);
		expect(h.queue.state()).toMatchObject({ pending: [], queued: [], offline: false, conflict: null });
		expect(h.storage.getItem(KEY)).toBeNull();
	});

	it('flush returns the outcome to a caller that asks', async () => {
		const h = offlineWith(['a'], [applied(grow(seen, ['a']))]);
		await expect(h.queue.flush()).resolves.toEqual({ status: 'applied' });
		const c = offlineWith(['a'], [conflict(grow(seen, ['b']))]);
		const out = await c.queue.flush();
		expect(out.status).toBe('conflict');
	});

	it('the online event flushes too, and taps made during a flush go in the next batch', async () => {
		let release!: (o: CommandOutcome) => void;
		const first = grow(seen, ['a'], 1);
		const h = offlineWith(['a'], [() => new Promise<CommandOutcome>((r) => (release = r)), applied(grow(first, ['b']))]);
		h.queue.reconnect();
		await tick();
		h.queue.point('b');
		// B can't join a request already out: it would change what that client_id means.
		expect(h.queue.state()).toMatchObject({ queued: [{ kind: 'point' }, { kind: 'point' }], sending: 1 });
		release({ status: 'applied', view: first });
		await tick();
		expect(h.sent.map((r) => [r.base_revision, r.commands])).toEqual([
			[seen.revision, [{ kind: 'point', team: 'a' }]],
			[first.revision, [{ kind: 'point', team: 'b' }]]
		]);
		expect(h.queue.state().queued).toEqual([]);
	});

	it('conflict_offers_keep_server_or_keep_mine', async () => {
		// Offline, the phone scored A, B; meanwhile the watch scored B on the server.
		const server = grow(seen, ['b']);
		const h = offlineWith(['a', 'b'], [conflict(server)]);
		const out = await h.queue.flush();
		if (out.status !== 'conflict') throw new Error(out.status);
		expect(out.conflict.server).toEqual(server);
		// Mine: the events up to the base seq, then my taps, scored by the engine.
		expect(points(out.conflict.mine)).toEqual(['a', 'a', 'b']);
		expect(out.conflict.mine?.score).toEqual(
			(replayLocal(seen.format, seen.start, [{ point: 'a' }, { point: 'a' }, { point: 'b' }]) as { ok: unknown }).ok
		);
		expect(h.queue.state().conflict).toEqual(out.conflict);
		expect(h.views).toEqual([server]);
		// Never merged: nothing more went to the server on its own.
		await tick(60_000);
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.replace).not.toHaveBeenCalled();

		// Keep the server's score: my taps go.
		h.queue.keepServer();
		expect(h.queue.state()).toMatchObject({ queued: [], conflict: null, offline: false });
		expect(h.storage.getItem(KEY)).toBeNull();
		expect(h.replace).not.toHaveBeenCalled();

		// Keep this phone's score: a replace from the base seq.
		const mine = harness([conflict(server)], {
			view: seen,
			replace: [applied(grow(server, []))]
		});
		mine.queue.connection('offline');
		mine.queue.point('a');
		mine.queue.point('b');
		await mine.queue.flush();
		await mine.queue.keepMine();
		expect(mine.replace).toHaveBeenCalledTimes(1);
		expect(mine.queue.state()).toMatchObject({ queued: [], conflict: null });
	});

	it('keep mine calls replace with the queued base seq and the conflict revision', async () => {
		const server = grow(seen, ['b', 'b'], 2);
		const replacedView = grow(seen, ['a', 'b'], 3);
		const h = offlineWith(['a', 'b'], [conflict(server)], { replace: [applied(replacedView)] });
		await h.queue.flush();
		const out = await h.queue.keepMine();
		expect(out).toEqual({ status: 'applied' });
		expect(h.replaced).toEqual([
			{
				client_id: 'id-2',
				base_revision: server.revision,
				base_seq: seen.head_seq,
				commands: [
					{ kind: 'point', team: 'a' },
					{ kind: 'point', team: 'b' }
				]
			}
		]);
		expect(h.replaced[0]?.client_id).not.toBe(h.sent[0]?.client_id);
		expect(h.views.at(-1)).toEqual(replacedView);
		expect(h.queue.state()).toMatchObject({ queued: [], conflict: null, offline: false });
		expect(h.storage.getItem(KEY)).toBeNull();
	});

	it('keep mine that conflicts again shows the choice again, with the fresh server view', async () => {
		const server = grow(seen, ['b']);
		const moved = grow(server, ['b']);
		const h = offlineWith(['a'], [conflict(server)], { replace: [conflict(moved), applied(grow(seen, ['a'], 4))] });
		await h.queue.flush();
		const again = await h.queue.keepMine();
		expect(again.status).toBe('conflict');
		expect(h.queue.state().conflict?.server).toEqual(moved);
		await tick(60_000);
		expect(h.replace).toHaveBeenCalledTimes(1);
		await h.queue.keepMine();
		expect(h.replaced.map((r) => r.base_revision)).toEqual([server.revision, moved.revision]);
		expect(h.replaced[1]?.base_seq).toBe(seen.head_seq);
		expect(h.replaced[0]?.client_id).not.toBe(h.replaced[1]?.client_id);
	});

	it('retry reuses the same client id', async () => {
		const done = grow(seen, ['a', 'b'], 1);
		const h = offlineWith(['a', 'b'], [offline, offline, applied(done)]);
		h.queue.connection('live');
		await tick();
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.queue.state().offline).toBe(true);
		await tick(1000);
		expect(h.post).toHaveBeenCalledTimes(2);
		// A reconnect while it backs off sends now, still the same request.
		h.queue.reconnect();
		await tick();
		expect(h.post).toHaveBeenCalledTimes(3);
		expect(new Set(h.sent.map((r) => JSON.stringify(r))).size).toBe(1);
		expect(h.sent[0]?.batch).toBe(true);
		expect(h.queue.state()).toMatchObject({ queued: [], offline: false });
	});

	it('a rejected batch keeps the taps and offers to discard them', async () => {
		const h = offlineWith(['a'], [async () => ({ status: 'rejected', kind: 'match_over', index: 0 })]);
		await h.queue.flush();
		expect(h.queue.state()).toMatchObject({
			queued: [{ kind: 'point', team: 'a' }],
			problem: { kind: 'rejected', error: 'match_over' }
		});
		// It is not resent on its own: the server keeps that answer for that client_id.
		h.queue.reconnect();
		await tick(60_000);
		expect(h.post).toHaveBeenCalledTimes(1);
		h.queue.discard();
		expect(h.queue.state()).toMatchObject({ queued: [], problem: null, offline: false });
		expect(h.storage.getItem(KEY)).toBeNull();
	});

	it('offline undo pops the queue first', async () => {
		const h = offlineWith(['a', 'b'], [hang]);
		h.queue.undo();
		expect(h.queue.state().queued).toEqual([{ kind: 'point', team: 'a' }]);
		h.queue.undo();
		expect(h.queue.state().queued).toEqual([]);
		// Nothing of mine left: the undo takes back the server's last point, queued for the batch.
		h.queue.undo();
		expect(h.queue.state().queued).toEqual([{ kind: 'undo' }]);
		expect(points(h.queue.localView(seen))).toEqual([]);
		h.queue.point('b');
		h.queue.reconnect();
		await tick();
		expect(h.sent[0]?.commands).toEqual([
			{ kind: 'undo', target_seq: seen.events.at(-1)?.seq },
			{ kind: 'point', team: 'b' }
		]);
		expect(h.dropped).toEqual([]);
	});

	it('an offline undo with nothing anywhere to undo is refused', async () => {
		const h = harness([], { view: undone(seen) });
		h.queue.connection('offline');
		h.queue.undo();
		expect(h.queue.state().queued).toEqual([]);
		expect(h.dropped).toEqual([{ intent: { kind: 'undo' }, reason: { kind: 'nothing' } }]);
	});

	it('an undo behind the point in flight when the line dropped undoes that point', async () => {
		const landed = grow(seen, ['a']);
		const h = harness([offline, applied(landed), hang], { view: seen });
		h.queue.point('a');
		await tick();
		h.queue.undo();
		h.queue.point('b');
		h.queue.reconnect();
		await tick();
		// A landed right on the view the taps were made on, so the batch builds on its answer.
		expect(h.sent[2]).toMatchObject({
			batch: true,
			base_revision: landed.revision,
			commands: [
				{ kind: 'undo', target_seq: landed.head_seq },
				{ kind: 'point', team: 'b' }
			]
		});
	});

	it('a live point that lands after another device scored is part of mine, not merged', async () => {
		// The watch scored B before the phone's resent A arrived (A appends live, in arrival order).
		const landed = grow(grow(seen, ['b']), ['a']);
		const h = harness([offline, applied(landed), conflict(landed)], {
			view: seen,
			replace: [applied(grow(seen, ['a', 'b'], 4))]
		});
		h.queue.point('a');
		await tick();
		h.queue.point('b');
		h.queue.reconnect();
		await tick();
		// The batch stays on the view the taps were made on, so it can't merge silently.
		expect(h.sent[2]).toMatchObject({ batch: true, base_revision: seen.revision });
		const c = h.queue.state().conflict;
		expect(c?.server).toEqual(landed);
		expect(points(c?.mine ?? null)).toEqual(['a', 'a', 'b']);
		await h.queue.keepMine();
		expect(h.replaced[0]).toMatchObject({
			base_seq: seen.head_seq,
			commands: [
				{ kind: 'point', team: 'a' },
				{ kind: 'point', team: 'b' }
			]
		});
	});

	it('queue survives reload', async () => {
		const storage = new MemoryStorage();
		const h = offlineWith(['a', 'b'], [offline], { storage });
		h.queue.undo();
		h.queue.point('b');
		h.queue.reconnect();
		await tick();
		const firstTry = h.sent[0];
		h.queue.stop();

		// The page reloads: a new queue on the same store, before any server view arrives.
		const done = grow(seen, ['a', 'b'], 1);
		const r = harness([applied(done)], { view: null, storage, idsFrom: 50 });
		expect(r.queue.state()).toMatchObject({
			queued: [
				{ kind: 'point', team: 'a' },
				{ kind: 'point', team: 'b' }
			],
			offline: true
		});
		expect(points(r.queue.localView(seen))).toEqual(['a', 'a', 'b']);
		r.queue.connection('live');
		await tick();
		// The request that may have reached the server goes again unchanged.
		expect(r.sent).toEqual([firstTry]);
		expect(r.queue.state().queued).toEqual([]);
		expect(storage.getItem(KEY)).toBeNull();
	});

	it('a live tap in flight survives reload and is resent unchanged', async () => {
		const storage = new MemoryStorage();
		const h = harness([hang], { view: seen, storage });
		h.queue.point('a');
		await tick();
		h.queue.stop();
		const r = harness([applied(grow(seen, ['a']))], { view: null, storage, idsFrom: 50 });
		expect(r.queue.state().pending).toEqual([{ intent: { kind: 'point', team: 'a' }, base: seen.revision, sent: true }]);
		await tick();
		expect(r.sent).toEqual(h.sent);
		expect(r.queue.state().pending).toEqual([]);
	});

	it('a conflict before reload asks again after it', async () => {
		const storage = new MemoryStorage();
		const server = grow(seen, ['b']);
		const h = offlineWith(['a'], [conflict(server)], { storage });
		await h.queue.flush();
		h.queue.stop();
		const r = harness([conflict(server)], { view: null, storage, idsFrom: 50 });
		r.queue.connection('live');
		await tick();
		expect(r.queue.state().conflict?.server).toEqual(server);
		expect(r.sent[0]).toMatchObject({ batch: true, base_revision: seen.revision });
	});

	it('a corrupt or unknown stored queue is discarded with a notice, never a crash', () => {
		for (const raw of ['{not json', JSON.stringify({ v: 2, queued: [] }), JSON.stringify({ v: 1, queued: 'x' })]) {
			const storage = new MemoryStorage();
			storage.setItem(KEY, raw);
			const r = harness([], { view: seen, storage });
			expect(r.queue.discardedOnLoad).toBe(true);
			expect(r.queue.state()).toMatchObject({ pending: [], queued: [], offline: false });
			expect(storage.getItem(KEY)).toBeNull();
		}
		const fine = harness([], { view: seen });
		expect(fine.queue.discardedOnLoad).toBe(false);
	});

	it('a store that throws keeps the queue in memory', async () => {
		const storage = new MemoryStorage();
		storage.setItem = () => {
			throw new DOMException('quota', 'QuotaExceededError');
		};
		const h = offlineWith(['a'], [applied(grow(seen, ['a']))], { storage });
		expect(h.queue.state().queued).toHaveLength(1);
		await h.queue.flush();
		expect(h.queue.state().queued).toEqual([]);
	});

	it('a batch the server refuses outright keeps the taps', async () => {
		const h = offlineWith(['a'], [
			async () => {
				throw new ApiError(400, 'invalid', 'bad');
			}
		]);
		await h.queue.flush();
		expect(h.queue.state()).toMatchObject({ queued: [{ kind: 'point' }], problem: { kind: 'failed', message: 'bad' } });
	});
});

describe('review fixes', () => {
	const fresh = asView(newMatch);

	it('unanswered_live_taps_when_going_offline_are_not_merged_silently', async () => {
		// The phone taps A, A; the first post hangs and times out, the second never went.
		let fail!: () => void;
		const first = () =>
			new Promise<CommandOutcome>((_, reject) => (fail = () => reject(new ApiError(0, 'network', 'timed out', true))));
		// Meanwhile the watch scores B, B; the phone's resent A lands after them: 15-30 on the server.
		const landed = grow(grow(fresh, ['b', 'b']), ['a']);
		const h = harness([first, applied(landed), conflict(landed)], { view: fresh });
		h.queue.point('a');
		h.queue.point('a');
		await tick();
		fail();
		await tick();
		// The tap that never went is queued on the base, not left to go live.
		expect(h.queue.state().pending.map((p) => p.sent)).toEqual([true]);
		expect(h.queue.state().queued).toEqual([{ kind: 'point', team: 'a' }]);
		h.queue.reconnect();
		await tick();
		expect(h.sent.map((r) => r.batch)).toEqual([false, false, true]);
		expect(h.sent[2]?.base_revision).toBe(fresh.revision);
		const c = h.queue.state().conflict;
		expect(c?.server).toEqual(landed);
		expect(points(c?.mine ?? null)).toEqual(['a', 'a']);
	});

	it('a lone live tap that lands after another device scored opens the choice', async () => {
		const landed = grow(grow(seen, ['b', 'b']), ['a']);
		const h = harness([offline, applied(landed)], { view: seen, replace: [applied(grow(seen, ['a'], 4))] });
		h.queue.point('a');
		await tick();
		const out = h.queue.reconnect();
		await tick();
		expect(out).toBeUndefined();
		const c = h.queue.state().conflict;
		expect(c?.server).toEqual(landed);
		expect(points(c?.mine ?? null)).toEqual(['a', 'a']);
		await h.queue.keepMine();
		expect(h.replaced[0]).toMatchObject({ base_seq: seen.head_seq, commands: [{ kind: 'point', team: 'a' }] });
		expect(h.queue.state()).toMatchObject({ conflict: null, offline: false });
	});

	it('a live tap that lands straight on its base after an offline spell is not a conflict', async () => {
		const h = harness([offline, applied(grow(seen, ['a']))], { view: seen });
		h.queue.point('a');
		await tick();
		h.queue.reconnect();
		await tick();
		expect(h.queue.state()).toMatchObject({ conflict: null, offline: false, pending: [], queued: [] });
	});

	it('keep_mine_matches_sheet_when_other_device_undid_below_base', async () => {
		// The phone saw A, B; offline it scores A. Meanwhile the watch undid B (below the base).
		const before = grow(seen, ['b']);
		const server = undone(before);
		const h = harness([conflict(server)], { view: before, replace: [applied(grow(server, ['b', 'a'], 2))] });
		h.queue.connection('offline');
		h.queue.point('a');
		await h.queue.flush();
		// The sheet shows this phone's score as the phone had it: A, B, then A.
		const mine = h.queue.state().conflict?.mine ?? null;
		expect(points(mine)).toEqual(['a', 'b', 'a']);
		await h.queue.keepMine();
		// Keep what both agree on (A), put the rest back: the server ends at what the sheet showed.
		expect(h.replaced[0]).toEqual({
			client_id: 'id-2',
			base_revision: server.revision,
			base_seq: seen.events[0]?.seq,
			commands: [
				{ kind: 'point', team: 'b' },
				{ kind: 'point', team: 'a' }
			]
		});
	});

	it('keep mine with an offline undo of a server point replaces from before it', async () => {
		const before = grow(seen, ['b']);
		const server = grow(before, ['a']);
		const h = harness([conflict(server)], { view: before, replace: [applied(grow(seen, [], 3))] });
		h.queue.connection('offline');
		h.queue.undo();
		await h.queue.flush();
		expect(points(h.queue.state().conflict?.mine ?? null)).toEqual(['a']);
		await h.queue.keepMine();
		// Mine is a prefix of the server's: back up one and put the last agreed event back.
		expect(h.replaced[0]).toMatchObject({ base_seq: 0, commands: [{ kind: 'point', team: 'a' }] });
	});

	it('keep mine with 250 queued taps goes as one batch and one replace', async () => {
		const adv = grow({ ...seen, format: { sets: { deuce: 'advantage', sets: 'one', deciding: 'full_set' } } }, []);
		const taps = Array.from({ length: 250 }, (_, i) => (i % 2 === 0 ? 'b' : 'a') as 'a' | 'b');
		const server = grow(adv, ['a']);
		const h = offlineWith(taps, [conflict(server)], { view: adv, replace: [applied(grow(adv, taps, 2))] });
		const out = await h.queue.flush();
		expect(out.status).toBe('conflict');
		expect(h.sent).toHaveLength(1);
		expect(h.sent[0]?.commands).toHaveLength(250);
		await expect(h.queue.keepMine()).resolves.toEqual({ status: 'applied' });
		expect(h.replaced).toHaveLength(1);
		expect(h.replaced[0]?.commands).toHaveLength(250);
		expect(h.queue.state().queued).toEqual([]);
	});

	it('refuses a tap past the most one request can carry', () => {
		const h = harness([], { view: seen });
		h.queue.connection('offline');
		for (let i = 0; i < MAX_COMMANDS + 1; i++) h.queue.point(i % 2 === 0 ? 'a' : 'b');
		expect(h.queue.state().queued).toHaveLength(MAX_COMMANDS);
		expect(h.dropped).toEqual([{ intent: { kind: 'point', team: 'a' }, reason: { kind: 'full' } }]);
	});

	it('flush settles when the batch turns out empty', async () => {
		const h = harness([], { view: undone(seen) });
		h.queue.connection('offline');
		h.queue.point('a');
		h.queue.undo();
		await expect(h.queue.flush()).resolves.toEqual({ status: 'idle' });
	});
});

describe('review fixes, round 2', () => {
	it('reload_with_flagged_inflight_job_still_asks', async () => {
		const storage = new MemoryStorage();
		const h = harness([offline], { view: seen, storage });
		h.queue.point('a');
		await tick();
		h.queue.stop();
		// Reloaded before the tap was answered; the watch scored B, B meanwhile.
		const landed = grow(grow(seen, ['b', 'b']), ['a']);
		const r = harness([applied(landed)], { view: null, storage, idsFrom: 50 });
		expect(r.queue.state().offline).toBe(true);
		await tick();
		expect(r.sent).toEqual(h.sent);
		const c = r.queue.state().conflict;
		expect(c?.server).toEqual(landed);
		expect(points(c?.mine ?? null)).toEqual(['a', 'a']);
	});

	it('reload_with_carried_only_conflict_restores_sheet', async () => {
		const storage = new MemoryStorage();
		const landed = grow(grow(seen, ['b', 'b']), ['a']);
		const h = harness([offline, applied(landed)], { view: seen, storage });
		h.queue.point('a');
		await tick();
		h.queue.reconnect();
		await tick();
		expect(h.queue.state().conflict).not.toBeNull();
		// Only a carried tap is left, and it is still worth keeping (and warning about).
		expect(h.queue.state().unsaved).toBe(true);
		expect(storage.getItem(KEY)).not.toBeNull();
		h.queue.stop();

		const r = harness([], { view: null, storage, idsFrom: 50, replace: [hang] });
		expect(r.queue.state()).toMatchObject({ offline: true, unsaved: true });
		r.queue.connection('live');
		await tick();
		// The page's first view arrives after the queue exists.
		r.setView(landed);
		r.queue.viewArrived();
		await tick();
		const c = r.queue.state().conflict;
		expect(c?.server).toEqual(landed);
		expect(points(c?.mine ?? null)).toEqual(['a', 'a']);
		// A carried-only replace on its way is still unsaved work.
		void r.queue.keepMine();
		expect(r.queue.state()).toMatchObject({ resolving: true, unsaved: true });
		expect(storage.getItem(KEY)).not.toBeNull();
	});

	it('carried_only_reload_without_view_does_not_recurse', async () => {
		const storage = new MemoryStorage();
		const landed = grow(grow(seen, ['b', 'b']), ['a']);
		const h = harness([offline, applied(landed)], { view: seen, storage });
		h.queue.point('a');
		await tick();
		h.queue.reconnect();
		await tick();
		h.queue.stop();

		const error = vi.spyOn(console, 'error').mockImplementation(() => {});
		try {
			const r = harness([], { view: null, storage, idsFrom: 50 });
			r.queue.connection('live');
			await tick();
			// Nothing to ask about without a view: it waits, without looping or a retry timer.
			expect(error).not.toHaveBeenCalled();
			expect(vi.getTimerCount()).toBe(0);
			expect(r.viewReads()).toBeLessThan(20);
			expect(r.queue.state().conflict).toBeNull();
			r.setView(landed);
			r.queue.viewArrived();
			await tick();
			expect(r.queue.state().conflict?.server).toEqual(landed);
			expect(error).not.toHaveBeenCalled();
		} finally {
			error.mockRestore();
		}
	});

	it('a stored v1 job without the offline flag loads as not offline', () => {
		const storage = new MemoryStorage();
		const req = { client_id: 'c1', batch: false, base_revision: 1, commands: [{ kind: 'point', team: 'a' }] };
		storage.setItem(
			KEY,
			JSON.stringify({
				v: 1,
				next: 2,
				jobs: [{ id: 1, intent: { kind: 'point', team: 'a' }, base: 1, req, after: null, target: null, conflicts: 0 }],
				queued: [],
				carried: [],
				base: null,
				outstanding: null
			})
		);
		const r = harness([hang], { view: seen, storage });
		expect(r.queue.discardedOnLoad).toBe(false);
		expect(r.queue.state()).toMatchObject({ offline: false, pending: [{ sent: true }] });
	});
});

describe('locked by the coordinator', () => {
	// The coordinator typed the result: the server refuses this phone's taps with 409 locked.
	const lockedView = (from = seen) => ({ ...grow(from, ['b', 'b']), locked: true });

	it('locked_drops_queue_with_note', async () => {
		const server = lockedView();
		const h = offlineWith(['a', 'b', 'a'], [locked(server)]);
		expect(h.storage.getItem(KEY)).not.toBeNull();
		const out = await h.queue.flush();
		expect(out).toEqual({ status: 'locked' });
		expect(h.queue.state()).toMatchObject({
			pending: [],
			queued: [],
			sending: 0,
			base: null,
			offline: false,
			conflict: null,
			problem: null,
			unsaved: false
		});
		expect(h.storage.getItem(KEY)).toBeNull();
		expect(h.views).toEqual([server]);
		expect(h.dropped).toEqual([{ intent: { kind: 'point', team: 'a' }, reason: { kind: 'locked', count: 3 } }]);
		// Nothing is resent.
		await tick(60_000);
		expect(h.post).toHaveBeenCalledTimes(1);
	});

	it('locked_never_opens_conflict', async () => {
		// Live: the tap out and the one waiting behind it both go.
		const server = lockedView();
		const h = harness([locked(server)], { view: seen });
		h.queue.point('a');
		h.queue.point('b');
		await tick(60_000);
		expect(h.post).toHaveBeenCalledTimes(1);
		expect(h.queue.state()).toMatchObject({ pending: [], queued: [], conflict: null, problem: null, unsaved: false });
		expect(h.dropped).toEqual([{ intent: { kind: 'point', team: 'a' }, reason: { kind: 'locked', count: 2 } }]);
		expect(h.views).toEqual([server]);
		expect(h.storage.getItem(KEY)).toBeNull();

		// "Keep this phone's score" answered locked: the choice closes, nothing is retried.
		const moved = grow(seen, ['b']);
		const c = offlineWith(['a'], [conflict(moved)], { replace: [locked(lockedView(moved))] });
		expect((await c.queue.flush()).status).toBe('conflict');
		await expect(c.queue.keepMine()).resolves.toEqual({ status: 'locked' });
		expect(c.queue.state()).toMatchObject({ queued: [], conflict: null, resolving: false, unsaved: false });
		expect(c.dropped).toEqual([{ intent: { kind: 'point', team: 'a' }, reason: { kind: 'locked', count: 1 } }]);
		expect(c.storage.getItem(KEY)).toBeNull();
		await tick(60_000);
		expect(c.replace).toHaveBeenCalledTimes(1);
	});

	it('a locked answer to a typed final drops the queue the same way', async () => {
		const server = lockedView();
		const h = offlineWith(['a', 'b'], []);
		h.queue.locked(server);
		expect(h.queue.state()).toMatchObject({ queued: [], offline: false, unsaved: false });
		expect(h.dropped).toEqual([{ intent: { kind: 'point', team: 'a' }, reason: { kind: 'locked', count: 2 } }]);
		expect(h.views).toEqual([server]);
		expect(h.storage.getItem(KEY)).toBeNull();
		// With nothing queued there is nothing to report.
		const empty = harness([], { view: seen });
		empty.queue.locked(server);
		expect(empty.dropped).toEqual([]);
		expect(empty.views).toEqual([server]);
	});
});

describe('local views', () => {
	const state = (over: Partial<QueueState>): QueueState => ({
		pending: [],
		queued: [],
		sending: 0,
		base: null,
		offline: true,
		conflict: null,
		resolving: false,
		problem: null,
		unsaved: false,
		...over
	});

	it('hides the taps of the batch in flight once a newer view holds them', () => {
		const s = state({ queued: [{ kind: 'point', team: 'b' }], sending: 1, base: seen.revision });
		expect(points(localView(seen, s, replayLocal))).toEqual(['a', 'b']);
		expect(points(localView(grow(seen, ['b']), s, replayLocal))).toEqual(['a', 'b']);
	});

	it('shows the server score under an open conflict choice', () => {
		const server = grow(seen, ['b']);
		const s = state({ queued: [{ kind: 'point', team: 'a' }], base: seen.revision, conflict: { server, mine: null } });
		expect(localView(server, s, replayLocal)).toBe(server);
	});

	it('shows the server view when the engine refuses the taps', () => {
		const over = asView(finished);
		expect(localView(over, state({ queued: [{ kind: 'point', team: 'a' }] }), replayLocal)).toBe(over);
		expect(mineView(over, over, [{ kind: 'point', team: 'a' }], replayLocal)).toBeNull();
	});

	it('shows the server view when the engine is not loaded', () => {
		const boom = () => {
			throw new Error('not loaded');
		};
		expect(localView(seen, state({ queued: [{ kind: 'point', team: 'a' }] }), boom)).toBe(seen);
	});
});

