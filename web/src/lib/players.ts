// The signed-in user's player list (crates/server/src/players_api.rs) and history, typed, plus
// the pure helpers the picker and the list page share.
//
// Every call goes through api.ts's accountRequest with `signedIn`, so it gets the account
// timeout, and a 401 (the session ended) sends the browser to sign-in.

import { accountRequest, ApiError } from './api';
import { authMessage } from './auth';
import { isHistoryPage, isPlayer, isPlayerList, isPlayerPage } from './guards';
import { MAX_NAME_LENGTH } from './setup';
import { strings } from './strings';
import type { FinalScore, HistoryEntry, Page, Player, PlayerChange, PlayerPage, PlayerRecord, SetScore } from './types';

const enc = encodeURIComponent;

/**
 * Every player, archived included, ordered by name. `quiet` turns off the 401 redirect to
 * sign-in, for a page that works signed out too (new match), where a failure just means no list.
 */
export async function listPlayers(opts: { quiet?: boolean } = {}): Promise<Player[]> {
	const list = await accountRequest<{ items: Player[] }>('/api/players', {
		signedIn: !opts.quiet,
		guard: isPlayerList
	});
	return list.items;
}

/**
 * Add a player by name. A name already on the list (in any case) gives the existing player
 * back (200, restored if it was archived), so either answer is the player to bind.
 */
export function addPlayer(name: string): Promise<Player> {
	return accountRequest('/api/players', {
		method: 'POST',
		body: { name },
		ok: [200, 201],
		signedIn: true,
		guard: isPlayer
	});
}

/** Rename, archive or restore. */
export function updatePlayer(id: string, change: PlayerChange): Promise<Player> {
	return accountRequest(`/api/players/${enc(id)}`, {
		method: 'PATCH',
		body: change,
		signedIn: true,
		guard: isPlayer
	});
}

const before = (cursor?: string | null) => (cursor ? `?before=${enc(cursor)}` : '');

/** Finished matches the user started, newest first; pass `next` back for the following page. */
export function getHistory(cursor?: string | null): Promise<Page<HistoryEntry>> {
	return accountRequest(`/api/me/history${before(cursor)}`, { signedIn: true, guard: isHistoryPage });
}

/** One player: their record, placings and matches (paged like history). */
export function getPlayerPage(id: string, cursor?: string | null): Promise<PlayerPage> {
	return accountRequest(`/api/players/${enc(id)}${before(cursor)}`, { signedIn: true, guard: isPlayerPage });
}

/** What to tell the user about a failed player call. */
export function playerMessage(e: unknown): string {
	const errors: Record<string, string> = strings.players.errors;
	if (e instanceof ApiError && Object.hasOwn(errors, e.code)) return errors[e.code]!;
	return authMessage(e);
}

// ---- pure helpers --------------------------------------------------------------------------

/** At most this many suggestions under a name field. */
export const MAX_SUGGESTIONS = 6;

/** How names compare, as the server's `name_key` does: trimmed, lowercased. */
export const nameKey = (name: string) => name.trim().toLocaleLowerCase();

export interface Suggestion {
	/** Active players not picked in another slot whose name contains the query; at most six. */
	matches: Player[];
	/** The one of `matches` whose name is the query. */
	exact: Player | null;
	/**
	 * The query can be added as a new player: not blank, at most 40 characters, and no active
	 * player has this name, picked or not. (Adding a picked name would give that player back, and
	 * the match would have the same player twice.) An archived name can be added: that restores it.
	 */
	canAdd: boolean;
}

export function suggest(players: Player[], query: string, taken: string[]): Suggestion {
	const key = nameKey(query);
	const active = players.filter((p) => !p.archived);
	const free = active.filter((p) => !taken.includes(p.id));
	const found = key === '' ? free : free.filter((p) => nameKey(p.name).includes(key));
	const exact = key === '' ? null : (found.find((p) => nameKey(p.name) === key) ?? null);
	const matches = (exact ? [exact, ...found.filter((p) => p !== exact)] : found).slice(0, MAX_SUGGESTIONS);
	const canAdd =
		key !== '' && Array.from(query.trim()).length <= MAX_NAME_LENGTH && !active.some((p) => nameKey(p.name) === key);
	return { matches, exact, canAdd };
}

/** "3 W · 1 L · 0 D". */
export function recordText(r: PlayerRecord): string {
	return strings.players.record(r.wins, r.losses, r.draws);
}

/**
 * A finished match's score on one line: "6-4 3-6 10-7" for sets, "7-5" timed.
 * A set won on a tie-break adds the loser's tie-break points, "7-6(5)"; a deciding super
 * tie-break (its games are 1-0, its points in `tiebreak`) reads as its points. A timed match
 * that went to overtime shows the tied games before the overtime game: "5-5, overtime".
 */
export function scoreText(score: FinalScore): string {
	if (score.timed || score.sets.length === 0) {
		if (score.overtime) return strings.history.overtimeScore(pair(score.regulation_games));
		return pair(score.games);
	}
	return score.sets.map(setText).join(' ');
}

const pair = ([a, b]: readonly [number, number]) => `${a}-${b}`;

function setText(set: SetScore): string {
	const tb = set.tiebreak;
	if (!tb) return pair([set.a, set.b]);
	if (set.a + set.b === 1) return pair(tb);
	return `${pair([set.a, set.b])}(${Math.min(tb[0], tb[1])})`;
}
