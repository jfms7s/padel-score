// The hand-off from "Start match" on a game to New match: the start route's answer waits in
// this tab's session storage, keyed by what the match starts from, until New match takes it.

import { ApiError } from './api';
import { boardMessage } from './boards';
import type { Setup } from './setup';
import { scheduledMessage } from './scheduled';
import { strings } from './strings';
import { safeSessionStorage } from './storage';
import type { StartPlayers, StartedFrom } from './types';

const key = (from: string) => `padel-score:start:${from}`;

/** The `from` query value for a source: "game:<id>" or "scheduled:<id>". */
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
	if (!Array.isArray(players) || players.length < 1 || players.length > 4) return false;
	if (!players.every((p) => typeof p?.player_id === 'string' && typeof p?.name === 'string')) return false;
	return (from?.kind === 'game' || from?.kind === 'scheduled') && typeof from.id === 'string';
}

/**
 * The setup with the game's players in their slots (seats 0-1 are team A, 2-3 team B). A short
 * game has fewer than four: the seats it lacks stay empty for the starter to fill. A scheduled
 * match brings its rules too.
 */
export function withStartPlayers(setup: Setup, s: StartPlayers): Setup {
	const seat = (i: number) => s.players[i];
	const name = (i: number) => seat(i)?.name ?? '';
	const id = (i: number) => seat(i)?.player_id ?? null;
	const rules = s.format !== null && 'sets' in s.format ? s.format.sets : null;
	return {
		...setup,
		...(rules ? { deuce: rules.deuce, sets: rules.sets, deciding: rules.deciding } : {}),
		teams: { a: [name(0), name(1)], b: [name(2), name(3)] },
		players: { a: [id(0), id(1)], b: [id(2), id(3)] }
	};
}

/** What a `from` query value starts from, or null when it is not "game:<id>" or "scheduled:<id>". */
export function parseFrom(raw: string | null): StartedFrom | null {
	const match = /^(game|scheduled):(.+)$/.exec(raw ?? '');
	return match ? { kind: match[1] as 'game' | 'scheduled', id: match[2] ?? '' } : null;
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

/**
 * What to tell the starter when New match could not create the match, and whether the game or
 * scheduled match it came from is out of reach (so a retry must not send it again).
 * `message: null` leaves the page's own wording; a plain create (no source) is never changed.
 */
export function startFailure(
	e: unknown,
	fromSource: boolean,
	kind: StartedFrom['kind'] = 'game'
): { message: string | null; dropFrom: boolean } {
	if (!fromSource || !(e instanceof ApiError)) return { message: null, dropFrom: false };
	if (kind === 'scheduled') {
		if (e.status === 404) return { message: strings.setup.matchGone, dropFrom: true };
		if (e.status !== 409) return { message: null, dropFrom: false };
		return { message: scheduledMessage(e), dropFrom: GONE_CODES.includes(e.code) };
	}
	if (e.status === 404) return { message: strings.setup.gameGone, dropFrom: true };
	if (e.status !== 409) return { message: null, dropFrom: false };
	return { message: boardMessage(e), dropFrom: e.code === 'already_started' || e.code === 'game_over' };
}

/** The codes after which the scheduled match cannot start a match any more. */
const GONE_CODES: readonly string[] = ['match_closed', 'already_started', 'game_over'];
