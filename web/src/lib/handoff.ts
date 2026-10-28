// The hand-off from "Start match" on a game to New match: the start route's answer waits in
// this tab's session storage, keyed by what the match starts from, until New match takes it.

import type { Setup } from './setup';
import { safeSessionStorage } from './storage';
import type { StartPlayers, StartedFrom } from './types';

const key = (from: string) => `padel-score:start:${from}`;

/** The `from` query value for a source: "game:<id>". */
export function fromParam(from: StartedFrom): string {
	return `${from.kind}:${from.id}`;
}

/** Keep the answer for New match. Storage that is missing or full just loses it. */
export function putStartHandoff(s: StartPlayers): void {
	try {
		safeSessionStorage().setItem(key(fromParam(s.from)), JSON.stringify(s));
	} catch {
		/* New match then says the hand-off expired */
	}
}

/** The answer kept for `from` (the raw "kind:id" query value), once: it is removed as it is read. */
export function takeStartHandoff(from: string): StartPlayers | null {
	try {
		const store = safeSessionStorage();
		const raw = store.getItem(key(from));
		if (raw === null) return null;
		store.removeItem(key(from));
		const value: unknown = JSON.parse(raw);
		return isStartPlayers(value) && fromParam(value.from) === from ? value : null;
	} catch {
		return null;
	}
}

function isStartPlayers(v: unknown): v is StartPlayers {
	if (typeof v !== 'object' || v === null) return false;
	const { players, from } = v as Partial<StartPlayers>;
	if (!Array.isArray(players) || players.length !== 4) return false;
	if (!players.every((p) => typeof p?.player_id === 'string' && typeof p?.name === 'string')) return false;
	return from?.kind === 'game' && typeof from.id === 'string';
}

/** The setup with the four players in their slots (seats 0-1 are team A, 2-3 team B). */
export function withStartPlayers(setup: Setup, s: StartPlayers): Setup {
	const [a0, a1, b0, b1] = s.players;
	return {
		...setup,
		teams: { a: [a0.name, a1.name], b: [b0.name, b1.name] },
		players: { a: [a0.player_id, a1.player_id], b: [b0.player_id, b1.player_id] }
	};
}

/** What a `from` query value starts from, or null when it is not "game:<id>". */
export function parseFrom(raw: string | null): StartedFrom | null {
	const match = /^game:(.+)$/.exec(raw ?? '');
	return match ? { kind: 'game', id: match[1] } : null;
}

/** What New match starts with for its `from` query value (null for a plain New match). */
export interface StartedSetup {
	setup: Setup;
	/** The source to send with the create, once its hand-off was found. */
	from: StartedFrom | null;
	/** `from` named a source but its hand-off is gone (a reload, another tab). */
	expired: boolean;
}

/** Take the hand-off for `raw` (once) and fill `last` with it. Call it once per `from` value. */
export function startFromParam(raw: string | null, last: Setup): StartedSetup {
	if (raw === null) return { setup: last, from: null, expired: false };
	const handoff = takeStartHandoff(raw);
	const source = parseFrom(raw);
	if (!handoff || !source) return { setup: last, from: null, expired: true };
	return { setup: withStartPlayers(last, handoff), from: handoff.from, expired: false };
}
