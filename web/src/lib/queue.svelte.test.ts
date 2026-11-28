import { flushSync } from 'svelte';
import { describe, expect, it } from 'vitest';
import { asView } from './fixtures/load';
import onePoint from './fixtures/view-one-point.json';
import { createQueue, queueKey, type Replay } from './queue';
import type { StorageLike } from './setup';
import type { MatchView } from './types';

// The scorer page creates its queue inside an effect. Whatever the queue reads from the page
// while it is being created must not become a dependency of that effect: every new view would
// then tear the queue down and build it again.

const seen = asView(onePoint);
const TOKEN = 'tok';
const KEY = queueKey(TOKEN);

class MemoryStorage implements StorageLike {
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

// The engine isn't loaded in this project; the effect's runs don't depend on the score.
const replay: Replay = () => ({ err: { kind: 'bad_json', message: 'no engine' } });
/** A newer server view, as the stream would bring. */
const newer = (n: number): MatchView => ({ ...seen, revision: seen.revision + n });

describe('queue created in an effect', () => {
	it('creating_a_queue_with_carried_taps_in_an_effect_does_not_rerun_it', () => {
		const storage = new MemoryStorage();
		// A reload after a batch was refused: only a carried tap is left in the store.
		storage.setItem(
			KEY,
			JSON.stringify({
				v: 1,
				next: 1,
				jobs: [],
				queued: [],
				carried: [{ kind: 'point', team: 'a' }],
				base: { revision: seen.revision, head_seq: seen.head_seq, events: seen.events },
				outstanding: null
			})
		);
		let view = $state<MatchView | null>(seen);
		let runs = 0;
		const stop = $effect.root(() => {
			$effect(() => {
				runs++;
				const q = createQueue(TOKEN, storage, { post: hang, replace: hang }, replay, {
					view: () => view,
					newClientId: () => 'id',
					setTimeout: (f, ms) => setTimeout(f, ms),
					clearTimeout: (t) => clearTimeout(t),
					onView: () => {},
					onDropped: () => {},
					onState: () => {}
				});
				return () => q.stop();
			});
		});
		flushSync();
		expect(runs).toBe(1);

		view = newer(1);
		flushSync();
		view = newer(2);
		flushSync();
		expect(runs).toBe(1);
		stop();
	});
});

function hang() {
	return new Promise<never>(() => {});
}
