// The last match view this device accepted, kept so a cold open without a line still shows the
// score (and the scorer can keep tapping, queued against it as the base).

import { isMatchView } from './guards';
import type { StorageLike } from './setup';
import type { MatchView } from './types';

const key = (token: string) => `padel-score:view:${token}`;

/** The stored view for `token`, or null when there is none or it can't be read as a MatchView. */
export function loadView(storage: StorageLike, token: string): MatchView | null {
	try {
		const raw = storage.getItem(key(token));
		if (raw === null) return null;
		const parsed: unknown = JSON.parse(raw);
		return isMatchView(parsed) ? parsed : null;
	} catch {
		return null;
	}
}

/** Remember `view`, unless a newer revision is already stored: storage never steps back. */
export function saveView(storage: StorageLike, token: string, view: MatchView): void {
	try {
		const stored = loadView(storage, token);
		if (stored && stored.revision >= view.revision) return;
		storage.setItem(key(token), JSON.stringify(view));
	} catch {
		// Full or blocked storage: the view just isn't remembered.
	}
}
