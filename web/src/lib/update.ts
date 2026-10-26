// When an open page switches to a newly deployed version. SvelteKit notices the new version
// (`updated` from $app/state); this decides when reloading is safe: never while taps are still
// on their way, and on the scoring screens only when nobody can be mid-tap.

import type { StorageLike } from './setup';

/** Scoring screens take taps; viewing screens only show; the rest is the app. */
export type UpdateScreen = 'scoring' | 'viewing' | 'app';

export function updateScreen(pathname: string): UpdateScreen {
	if (/^\/(s|c)\//.test(pathname)) return 'scoring';
	if (/^\/(v|cv|e)\//.test(pathname)) return 'viewing';
	return 'app';
}

/** How long after the page comes back into view a found update still counts as "on return". */
export const SHOWN_MS = 3_000;
/** At most one automatic reload in this long, so a stale proxy or cache can't make a loop. */
export const RELOAD_GAP_MS = 60_000;

export interface UpdateMoment {
	screen: UpdateScreen;
	/** Something on the page isn't settled on the server, or a sheet is mid-entry. */
	held: boolean;
	/** The page is out of view: nobody can be tapping it. */
	hidden: boolean;
	/** The page came back into view within SHOWN_MS: the scorer hasn't started tapping yet. */
	justShown: boolean;
}

/**
 * Whether to reload right now, with a new version found. Never while held. A viewing screen
 * reloads at once; the app waits for its next navigation (see `beforeNavigate`) or a return,
 * so a half-filled form isn't wiped; a scoring screen only out of view or on return.
 */
export function reloadNow(m: UpdateMoment): boolean {
	if (m.held) return false;
	if (m.hidden || m.justShown) return true;
	return m.screen === 'viewing';
}

const KEY = 'update-reloaded-at';

/** Whether an automatic reload is allowed: none in the last RELOAD_GAP_MS (wall clock, ms). */
export function mayReload(storage: StorageLike, now: number): boolean {
	let at: number;
	try {
		at = Number(storage.getItem(KEY));
	} catch {
		return true;
	}
	return !Number.isFinite(at) || at <= 0 || now - at >= RELOAD_GAP_MS || now < at;
}

/** Notes an automatic reload, for `mayReload`. Never throws. */
export function markReload(storage: StorageLike, now: number): void {
	try {
		storage.setItem(KEY, String(now));
	} catch {
		// Blocked: the gap just isn't enforced.
	}
}
