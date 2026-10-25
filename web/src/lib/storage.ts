import type { StorageLike } from './setup';

const nothing: StorageLike = { getItem: () => null, setItem: () => {}, removeItem: () => {} };

/** `localStorage`, or a store that remembers nothing when merely touching the global throws
 *  (blocked site data, some private modes, sandboxed frames). */
export function safeStorage(): StorageLike {
	try {
		const s = globalThis.localStorage;
		return s ?? nothing;
	} catch {
		return nothing;
	}
}
