// The quick-match setup: the form's model, the request it becomes, and what this device
// remembers (the last rules and the recently started matches). Storage is injected so the
// functions test against a fake, and every access is wrapped because storage can be missing,
// full or blocked.

import { strings } from './strings';
import type { CreateMatchRequest, DecidingSet, DeuceRule, Player, SetsFormat, Teams } from './types';

export interface StorageLike {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

export interface Setup {
	deuce: DeuceRule;
	sets: SetsFormat;
	deciding: DecidingSet;
	teams: Teams;
	firstServer: Player;
	/** The index (0 or 1) of the player who serves first for the pair that receives first. */
	otherFirstServer: 0 | 1;
}

export interface RecentMatch {
	scorer_token: string;
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
		firstServer: { team: 'a', index: 0 },
		otherFirstServer: 0
	};
}

/** Changing the number of sets; one set can only have a full deciding set. */
export function setSets(setup: Setup, sets: SetsFormat): Setup {
	return { ...setup, sets, deciding: sets === 'one' ? 'full_set' : setup.deciding };
}

/** A player's name, or "Player N" (N = 1..4 across team A then team B) while it is blank. */
function nameOf(setup: Setup, team: 'a' | 'b', index: number): string {
	const typed = setup.teams[team][index]?.trim() ?? '';
	return typed || strings.setup.player((team === 'a' ? 0 : 2) + index + 1);
}

export function serveChoices(setup: Setup): {
	first: { player: Player; label: string }[];
	other: { index: 0 | 1; label: string }[];
} {
	const first = (['a', 'b'] as const).flatMap((team) =>
		([0, 1] as const).map((index) => ({ player: { team, index }, label: nameOf(setup, team, index) }))
	);
	const otherTeam = setup.firstServer.team === 'a' ? 'b' : 'a';
	const other = ([0, 1] as const).map((index) => ({ index, label: nameOf(setup, otherTeam, index) }));
	return { first, other };
}

export function toCreateRequest(setup: Setup): CreateMatchRequest {
	const name = (team: 'a' | 'b', i: 0 | 1) => nameOf(setup, team, i);
	return {
		format: {
			sets: {
				deuce: setup.deuce,
				sets: setup.sets,
				deciding: setup.sets === 'one' ? 'full_set' : setup.deciding
			}
		},
		start: { first_server: setup.firstServer, other_first_server: setup.otherFirstServer },
		teams: { a: [name('a', 0), name('a', 1)], b: [name('b', 0), name('b', 1)] }
	};
}

export function matchLabel(teams: Teams): string {
	return `${strings.teamName(...teams.a)} v ${strings.teamName(...teams.b)}`;
}

// ---- storage ---------------------------------------------------------------------------

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const oneOf = <T>(list: readonly T[], v: unknown): v is T => list.includes(v as T);
const isNames = (v: unknown): v is [string, string] =>
	Array.isArray(v) && v.length === 2 && v.every((n) => typeof n === 'string');

function parseSetup(raw: unknown): Setup | null {
	if (!isObject(raw)) return null;
	const { deuce, sets, deciding, teams, firstServer, otherFirstServer } = raw;
	if (!oneOf(DEUCE_RULES, deuce) || !oneOf(SETS_FORMATS, sets) || !oneOf(DECIDING_SETS, deciding)) return null;
	if (!isObject(teams) || !isNames(teams.a) || !isNames(teams.b)) return null;
	if (!isObject(firstServer) || !oneOf(['a', 'b'], firstServer.team) || !oneOf([0, 1], firstServer.index)) return null;
	if (!oneOf([0, 1], otherFirstServer)) return null;
	return {
		deuce,
		sets,
		deciding: sets === 'one' ? 'full_set' : deciding,
		teams: { a: teams.a, b: teams.b },
		firstServer: { team: firstServer.team as 'a' | 'b', index: firstServer.index as number },
		otherFirstServer: otherFirstServer as 0 | 1
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
		storage.setItem(LAST_SETUP_KEY, JSON.stringify(setup));
	} catch {
		// Remembering is a convenience; a full or blocked store must not stop a match.
	}
}

const MAX_TIME = 8.64e15; // the largest time a Date can hold

const isRecent = (v: unknown): v is RecentMatch =>
	isObject(v) &&
	typeof v.scorer_token === 'string' &&
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

export const MAX_NAME_LENGTH = 40;

export function scorerPath(token: string): string {
	return `/s/${encodeURIComponent(token)}`;
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
	teams: Teams;
	now: number;
	go: (path: string) => Promise<unknown>;
}): Promise<{ navigated: boolean; path: string }> {
	const path = scorerPath(opts.scorerToken);
	try {
		saveLastSetup(opts.storage, opts.setup);
		addRecent(opts.storage, {
			scorer_token: opts.scorerToken,
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
