// Pages hold back a version update while something on them isn't settled (see update.ts).

let count = $state(0);

/** Holds updates until the returned release is called (once; later calls do nothing). */
export function holdUpdates(): () => void {
	count++;
	let released = false;
	return () => {
		if (released) return;
		released = true;
		count--;
	};
}

/** Whether any page holds updates back (reactive). */
export function updatesHeld(): boolean {
	return count > 0;
}
