import { flushSync } from 'svelte';
import { describe, expect, it } from 'vitest';
import { holdUpdates, updatesHeld } from './holds.svelte.ts';

describe('holdUpdates', () => {
	it('can be taken and released by an effect without re-running it', () => {
		let runs = 0;
		let held = $state(true);
		const stop = $effect.root(() => {
			$effect(() => {
				runs++;
				if (held) return holdUpdates();
			});
		});
		flushSync();
		expect(updatesHeld()).toBe(true);
		expect(runs).toBe(1);

		held = false;
		flushSync();
		expect(updatesHeld()).toBe(false);
		expect(runs).toBe(2);
		stop();
	});

	it('stays held until every hold is released', () => {
		const first = holdUpdates();
		const second = holdUpdates();
		first();
		first();
		expect(updatesHeld()).toBe(true);
		second();
		expect(updatesHeld()).toBe(false);
	});
});
