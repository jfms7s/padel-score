// The quick-match setup: the form's model, the request it becomes, and what this device
// remembers (the last rules and the recently started matches). Storage is injected so the
// functions test against a fake, and every access is wrapped because storage can be missing,
// full or blocked.

import { defaultServeOrder, fromStart, toStart, type ServeOrder } from './serve';
import { strings } from './strings';
import type { CreateMatchRequest, DecidingSet, DeuceRule, PlayerSlots, SetsFormat, Team, Teams } from './types';

export interface StorageLike {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
	removeItem(key: string): void;
}

export interface Setup {
	deuce: DeuceRule;
	sets: SetsFormat;
	deciding: DecidingSet;
	teams: Teams;
	serve: ServeOrder;
	/**
	 * The players picked from the signed-in user's list, by slot. Never remembered: a shared
	 * phone must not offer one coordinator's players to the next.
	 */
	players: PlayerSlots;
}

export function noPlayers(): PlayerSlots {
	return { a: [null, null], b: [null, null] };
}

export interface RecentMatch {
	scorer_token: string;
	/** Missing on entries saved before the share sheet needed it. */
	spectator_token?: string;
	label: string;
	created_at: number;
}

export const LAST_SETUP_KEY = 'padel-score:last-setup';
export const RECENT_KEY = 'padel-score:recent';
export const MAX_RECENT = 10;

export const DEUCE_RULES: DeuceRule[] = ['advantage', 'golden_point', 'star_point'];
export const SETS_FORMATS: SetsFormat[] = ['one', 'best_of_3'];
export const DECIDING_SETS: DecidingSet[] = ['full_set', 'super_tie_break'];

export function defaultSetup(): Setup {
	return {
		deuce: 'golden_point',
		sets: 'one',
		deciding: 'full_set',
		teams: { a: ['', ''], b: ['', ''] },
		serve: defaultServeOrder(),
		players: noPlayers()
	};
}

/** Changing the number of sets; one set can only have a full deciding set. */
export function setSets(setup: Setup, sets: SetsFormat): Setup {
	return { ...setup, sets, deciding: sets === 'one' ? 'full_set' : setup.deciding };
}

/** A slot's stand-in name, "Player N" (N = 1..4 across team A then team B). */
export function slotName(team: Team, index: number): string {
	return strings.setup.player((team === 'a' ? 0 : 2) + index + 1);
}

/** A player's name as entered, or their slot's stand-in while it is blank. */
export function nameOrSlot(teams: Teams, team: Team, index: 0 | 1): string {
	return teams[team][index].trim() || slotName(team, index);
}

export function toCreateRequest(setup: Setup): CreateMatchRequest {
	const name = (team: Team, i: 0 | 1) => nameOrSlot(setup.teams, team, i);
	const req: CreateMatchRequest = {
		format: {
			sets: {
				deuce: setup.deuce,
				sets: setup.sets,
				deciding: setup.sets === 'one' ? 'full_set' : setup.deciding
			}
		},
		start: toStart(setup.serve),
		teams: { a: [name('a', 0), name('a', 1)], b: [name('b', 0), name('b', 1)] }
	};
	const { a, b } = setup.players;
	if ([...a, ...b].some((id) => id !== null)) req.players = { a: [a[0], a[1]], b: [b[0], b[1]] };
	return req;
}

export function matchLabel(teams: Teams): string {
	return `${strings.teamName(...teams.a)} v ${strings.teamName(...teams.b)}`;
}

// ---- storage ---------------------------------------------------------------------------

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const oneOf = <T>(list: readonly T[], v: unknown): v is T => list.includes(v as T);
const isNames = (v: unknown): v is [string, string] =>
	Array.isArray(v) && v.length === 2 && v.every((n) => typeof n === 'string');

/**
 * The remembered serve choice: `serveTeam` + `serveFirst` today, `firstServer` +
 * `otherFirstServer` (the engine's start) before. Null if what is there is invalid; a setup
 * with neither shape gets the default.
 */
function parseServe(raw: Record<string, unknown>): ServeOrder | null {
	const { serveTeam, serveFirst, firstServer, otherFirstServer } = raw;
	if (serveTeam !== undefined || serveFirst !== undefined) {
		if (!oneOf(['a', 'b'], serveTeam) || !isObject(serveFirst)) return null;
		if (!oneOf([0, 1], serveFirst.a) || !oneOf([0, 1], serveFirst.b)) return null;
		return { team: serveTeam as Team, first: { a: serveFirst.a as 0 | 1, b: serveFirst.b as 0 | 1 } };
	}
	if (firstServer === undefined && otherFirstServer === undefined) return defaultServeOrder();
	if (!isObject(firstServer) || !oneOf(['a', 'b'], firstServer.team) || !oneOf([0, 1], firstServer.index)) return null;
	if (!oneOf([0, 1], otherFirstServer)) return null;
	return fromStart({
		first_server: { team: firstServer.team as Team, index: firstServer.index as number },
		other_first_server: otherFirstServer as number
	});
}

function parseSetup(raw: unknown): Setup | null {
	if (!isObject(raw)) return null;
	const { deuce, sets, deciding, teams } = raw;
	if (!oneOf(DEUCE_RULES, deuce) || !oneOf(SETS_FORMATS, sets) || !oneOf(DECIDING_SETS, deciding)) return null;
	if (!isObject(teams) || !isNames(teams.a) || !isNames(teams.b)) return null;
	const serve = parseServe(raw);
	if (!serve) return null;
	return {
		deuce,
		sets,
		deciding: sets === 'one' ? 'full_set' : deciding,
		teams: { a: teams.a, b: teams.b },
		serve,
		players: noPlayers()
	};
}

export function loadLastSetup(storage: StorageLike): Setup {
	try {
		const raw = storage.getItem(LAST_SETUP_KEY);
		if (raw !== null) return parseSetup(JSON.parse(raw)) ?? defaultSetup();
	} catch {
		// Unavailable or unreadable: start from the defaults.
	}
	return defaultSetup();
}

export function saveLastSetup(storage: StorageLike, setup: Setup): void {
	try {
		const { players: _notRemembered, serve, ...rest } = setup;
		storage.setItem(LAST_SETUP_KEY, JSON.stringify({ ...rest, serveTeam: serve.team, serveFirst: serve.first }));
	} catch {
		// Remembering is a convenience; a full or blocked store must not stop a match.
	}
}

const MAX_TIME = 8.64e15; // the largest time a Date can hold

const isRecent = (v: unknown): v is RecentMatch =>
	isObject(v) &&
	typeof v.scorer_token === 'string' &&
	(v.spectator_token === undefined || typeof v.spectator_token === 'string') &&
	typeof v.label === 'string' &&
	typeof v.created_at === 'number' &&
	Number.isFinite(v.created_at) &&
	Math.abs(v.created_at) <= MAX_TIME;

/** Recent matches, newest first. */
export function listRecent(storage: StorageLike): RecentMatch[] {
	try {
		const raw = storage.getItem(RECENT_KEY);
		const parsed: unknown = raw === null ? [] : JSON.parse(raw);
		return Array.isArray(parsed) ? parsed.filter(isRecent).slice(0, MAX_RECENT) : [];
	} catch {
		return [];
	}
}

export function addRecent(storage: StorageLike, entry: RecentMatch): void {
	try {
		const rest = listRecent(storage).filter((r) => r.scorer_token !== entry.scorer_token);
		storage.setItem(RECENT_KEY, JSON.stringify([entry, ...rest].slice(0, MAX_RECENT)));
	} catch {
		// As above.
	}
}

/** Forget a recent match on this device. */
export function removeRecent(storage: StorageLike, scorerToken: string): void {
	try {
		const rest = listRecent(storage).filter((r) => r.scorer_token !== scorerToken);
		storage.setItem(RECENT_KEY, JSON.stringify(rest));
	} catch {
		// As above.
	}
}

/** The spectator token remembered for a scorer token, if this device started the match. */
export function recentSpectator(storage: StorageLike, scorerToken: string): string | null {
	return listRecent(storage).find((r) => r.scorer_token === scorerToken)?.spectator_token ?? null;
}

export const MAX_NAME_LENGTH = 40;

/**
 * The fence screen's path. The spectator token rides in the fragment (`#v=`), which the
 * browser never sends to the server: MatchView doesn't carry it, and the scorer page needs it
 * for the share sheet, on this device after a reload and on any device the link is shared to.
 */
export function scorerPath(token: string, spectatorToken?: string): string {
	const path = `/s/${encodeURIComponent(token)}`;
	return spectatorToken ? `${path}#v=${encodeURIComponent(spectatorToken)}` : path;
}

/**
 * What happens once the server has created the match: remember it, then go to it. The match
 * exists whatever happens here, so nothing in this step may fail the creation; if navigation
 * itself fails the caller gets the path to offer as a link.
 */
export async function afterCreate(opts: {
	storage: StorageLike;
	setup: Setup;
	scorerToken: string;
	spectatorToken: string;
	teams: Teams;
	now: number;
	go: (path: string) => Promise<unknown>;
}): Promise<{ navigated: boolean; path: string }> {
	const path = scorerPath(opts.scorerToken, opts.spectatorToken);
	try {
		saveLastSetup(opts.storage, opts.setup);
		addRecent(opts.storage, {
			scorer_token: opts.scorerToken,
			spectator_token: opts.spectatorToken,
			label: matchLabel(opts.teams),
			created_at: opts.now
		});
	} catch {
		// Remembering is a convenience.
	}
	try {
		await opts.go(path);
		return { navigated: true, path };
	} catch {
		return { navigated: false, path };
	}
}
