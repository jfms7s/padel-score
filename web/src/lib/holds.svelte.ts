// Pages hold back a version update while something on them isn't settled (see update.ts).
import { untrack } from 'svelte';

let count = $state(0);

/**
 * Holds updates until the returned release is called (once; later calls do nothing). Pages call
 * it from an effect, so the count is changed untracked: reading it there would re-run the effect,
 * whose cleanup releases and takes the hold again, without end.
 */
export function holdUpdates(): () => void {
	untrack(() => count++);
	let released = false;
	return () => {
		if (released) return;
		released = true;
		untrack(() => count--);
	};
}

/** Whether any page holds updates back (reactive). */
export function updatesHeld(): boolean {
	return count > 0;
}
