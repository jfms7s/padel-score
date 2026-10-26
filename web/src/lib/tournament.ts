// The tournament's bracket (phase 7): the shapes of the server's `kind_data`, the pure helpers
// the bracket screens share (sections, slot and court texts, scores), and the coordinator's
// three bracket calls. Kept apart from `types.ts`, which phase 6 owns.

import { accountRequest } from './api';
import { teamText } from './events';
import { strings } from './strings';
import type { CourtOut, EventDetail, EventMatch, EventTeam, EventViewMatch, Place } from './types';

// ---- the wire shapes ---------------------------------------------------------------------

/** Where a match sits: draw `(place_from, size)`, match `idx` of it. */
export interface BracketPos {
	place_from: number;
	size: number;
	idx: number;
}

/** Where a slot's team comes from; `from` is null only in the first round of the main draw. */
export type BracketSource = { winner_of: BracketPos } | { loser_of: BracketPos };

export interface BracketNode extends BracketPos {
	label: string;
	/** Team ids, null until known. */
	teams: [string | null, string | null];
	from: [BracketSource | null, BracketSource | null];
	/** The event match, null until both teams are known. */
	match_id: string | null;
	/** Its winner changed after a later match started: the coordinator decides. */
	held: boolean;
	/** Coordinator only, on a held node: the labels of the matches Replay would discard. */
	replay_discards?: string[];
}

/** An event's `kind_data` for a tournament once it has started. */
export interface TournamentData {
	full_placement: boolean;
	teams: number;
	/** In the server's order: draws by size descending, then place_from, then idx. */
	nodes: BracketNode[];
	/** The computed places once the bracket is complete. */
	places: Place[] | null;
	/** Coordinator only. */
	can_redraw?: boolean;
}

/** `kind_data` as `TournamentData`, or null when it isn't one (setup, junk). */
export function tournamentData(kindData: unknown): TournamentData | null {
	if (typeof kindData !== 'object' || kindData === null) return null;
	if (!Array.isArray((kindData as { nodes?: unknown }).nodes)) return null;
	return kindData as TournamentData;
}

// ---- sections ----------------------------------------------------------------------------

export interface Column {
	title: string;
	nodes: BracketNode[];
}

export interface Section {
	kind: 'main' | 'placement';
	title: string | null;
	columns: Column[];
}

const t = strings.tournament;

const bySize = (nodes: BracketNode[]): Map<number, BracketNode[]> => {
	const map = new Map<number, BracketNode[]>();
	for (const n of [...nodes].sort((a, b) => b.size - a.size || a.place_from - b.place_from || a.idx - b.idx)) {
		map.set(n.size, [...(map.get(n.size) ?? []), n]);
	}
	return map;
};

function mainTitle(size: number): string {
	if (size >= 32) return t.roundOf32;
	if (size === 16) return t.roundOf16;
	if (size === 8) return t.quarterFinals;
	if (size === 4) return t.semiFinals;
	return t.finalAndThird;
}

function placementTitle(size: number, nodes: BracketNode[]): string {
	if (size <= 2) {
		const froms = [...new Set(nodes.map((n) => n.place_from))].sort((a, b) => a - b);
		return t.placesColumn(froms.map(ordinal));
	}
	if (size === 4) return t.placementSemis;
	if (size === 8) return t.placementQuarterFinals;
	return t.placementRoundOf16;
}

function sectionTitle(poolSize: number): string {
	if (poolSize <= 4) return t.placementSectionSemis;
	if (poolSize === 8) return t.placementSectionQuarters;
	return t.placementSectionOf16;
}

/** The bracket as the screens draw it: the main block, then one block per loser pool. */
export function bracketSections(d: TournamentData): Section[] {
	const mainNodes = d.nodes.filter((n) => n.place_from === 1 || (n.place_from === 3 && n.size === 2));
	const main: Section = {
		kind: 'main',
		title: null,
		columns: [...bySize(mainNodes)].map(([size, nodes]) => ({ title: mainTitle(size), nodes }))
	};
	const sections = [main];
	if (!d.full_placement) return sections;
	const mainSizes = [...new Set(d.nodes.filter((n) => n.place_from === 1).map((n) => n.size))].filter((s) => s >= 8);
	for (const size of mainSizes.sort((a, b) => a - b)) {
		const first = size / 2 + 1;
		const last = size;
		const pool = d.nodes.filter((n) => n.place_from >= first && n.place_from + n.size - 1 <= last);
		sections.push({
			kind: 'placement',
			title: sectionTitle(size / 2),
			columns: [...bySize(pool)].map(([s, nodes]) => ({ title: placementTitle(s, nodes), nodes }))
		});
	}
	return sections;
}

/** 1st, 2nd, 3rd, 4th, 11th, 21st. */
function ordinal(n: number): string {
	const tens = n % 100;
	if (tens >= 11 && tens <= 13) return `${n}th`;
	return `${n}${['th', 'st', 'nd', 'rd'][n % 10] ?? 'th'}`;
}

// ---- texts -------------------------------------------------------------------------------

const same = (a: BracketPos, b: BracketPos) => a.place_from === b.place_from && a.size === b.size && a.idx === b.idx;
const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

/**
 * What a slot of a bracket match shows: its team; else, when the match it comes from has both
 * teams, "A or B"; else "Winner of semi-final 1" or "Loser of quarter-final 3".
 */
export function slotText(
	d: TournamentData,
	node: BracketNode,
	side: 0 | 1,
	teams: readonly EventTeam[]
): { text: string; known: boolean } {
	const name = (id: string) => {
		const found = teams.find((x) => x.id === id);
		return found ? teamText(found) : strings.events.unknownTeam;
	};
	const id = node.teams[side];
	if (id !== null) return { text: name(id), known: true };
	const from = node.from[side];
	if (from === null) return { text: t.toBeDrawn, known: false };
	const pos = 'winner_of' in from ? from.winner_of : from.loser_of;
	const source = d.nodes.find((n) => same(n, pos));
	if (!source) return { text: t.toBeDrawn, known: false };
	const [a, b] = source.teams;
	if (a !== null && b !== null) return { text: t.eitherOr(name(a), name(b)), known: false };
	const label = lowerFirst(source.label);
	return { text: 'winner_of' in from ? t.winnerOf(label) : t.loserOf(label), known: false };
}

type MatchWhere = Pick<EventMatch, 'id' | 'court_id' | 'state'>;
type CourtWhere = Pick<CourtOut, 'id' | 'name' | 'current'>;

const isFinal = (n: Pick<BracketNode, 'place_from' | 'size'>) => n.place_from === 1 && n.size === 2;

/**
 * The server keeps the final off the courts while any other bracket node has no match yet or an
 * unfinished match on no court (a 5th/7th-place match can still be made after it).
 */
function finalHeldBack(d: TournamentData, matches: readonly MatchWhere[]): boolean {
	return d.nodes.some((n) => {
		if (isFinal(n)) return false;
		if (n.match_id === null) return true;
		const m = matches.find((x) => x.id === n.match_id);
		return m !== undefined && m.state !== 'finished' && m.court_id === null;
	});
}

/**
 * The strip under a bracket match: where it is playing or will play, null when finished or not
 * made. Pass the bracket `d` so the final reads "After the other matches" while it is held back.
 */
export function whereText(
	node: Pick<BracketNode, 'match_id' | 'place_from' | 'size'>,
	matches: readonly MatchWhere[],
	courts: readonly CourtWhere[],
	d?: TournamentData
): { text: string; live: boolean } | null {
	if (node.match_id === null) return null;
	const match = matches.find((m) => m.id === node.match_id);
	if (!match || match.state === 'finished') return null;
	const court = match.court_id === null ? undefined : courts.find((c) => c.id === match.court_id);
	if (!court) {
		const held = d !== undefined && isFinal(node) && finalHeldBack(d, matches);
		return { text: held ? t.afterOtherMatches : t.nextFreeCourt, live: false };
	}
	if (court.current === match.id && match.state === 'live') return { text: t.liveOn(court.name), live: true };
	return { text: t.nextOn(court.name), live: false };
}

type MatchScore = Pick<EventMatch, 'result' | 'live'> | Pick<EventViewMatch, 'result' | 'live'>;

/** One side's games, set by set ("6", "7 3 10"): the result's, else the live ones, else "". */
export function sideScore(m: MatchScore, side: 'a' | 'b'): string {
	const i = side === 'a' ? 0 : 1;
	if (m.result) {
		const { sets, games } = m.result;
		if (sets.length === 0) return String(games[i]);
		// A deciding super tie-break is stored as 1-0 with its points in `tiebreak`.
		return sets.map((s) => (s.tiebreak && s.a + s.b === 1 ? s.tiebreak[i] : s[side])).join(' ');
	}
	if (m.live) {
		const { sets, games } = m.live;
		return sets.length === 0 ? String(games[i]) : sets.map((s) => s[side]).join(' ');
	}
	return '';
}

/** Why the team count can't start, in words; null when 4, 8, 16 or 32. */
export function teamCountHint(n: number): string | null {
	const sizes = [4, 8, 16, 32];
	if (sizes.includes(n)) return null;
	const down = sizes.filter((s) => s < n).at(-1) ?? null;
	const up = sizes.find((s) => s > n) ?? null;
	return t.teamCountHint(n, down, up);
}

/** The lines of a held node's warning card; `replay` is null when Replay isn't on offer. */
export interface HeldText {
	warning: string;
	replay: string | null;
	keep: string;
	finishedNote: string | null;
}

/**
 * What a held node tells the coordinator: the warning, the Replay and Keep lines. After Finish
 * no match can be made, so only Keep is offered.
 */
export function heldText(
	d: TournamentData,
	node: BracketNode,
	finished: boolean,
	matches: readonly Pick<EventMatch, 'id' | 'state'>[] = []
): HeldText {
	const discards = node.replay_discards ?? [];
	// Replay also withdraws matches nobody has started: name the first one that has begun.
	const started = discards.find((label) => {
		const id = d.nodes.find((n) => n.label === label)?.match_id;
		const state = id == null ? undefined : matches.find((m) => m.id === id)?.state;
		return state === 'live' || state === 'finished';
	});
	return {
		warning: t.heldWarning(node.label, started ?? null),
		replay: finished || discards.length === 0 ? null : t.replayDiscards(discards),
		keep: t.keepNote(node.label),
		finishedNote: finished ? t.heldFinished : null
	};
}

/** Why the bracket can't be finished: matches to play, or a held result; null when it can. */
export function finishBlocker(d: TournamentData): 'unfinished' | 'decision' | null {
	if (d.nodes.some((n) => n.held)) return 'decision';
	return d.places === null ? 'unfinished' : null;
}

type CourtFree = Pick<CourtOut, 'id' | 'name' | 'current' | 'queue'>;

/** Some court is free (nothing live or queued) while some bracket match shows "Next free court". */
export function canFillCourts(
	d: TournamentData,
	matches: readonly MatchWhere[],
	courts: readonly CourtFree[]
): boolean {
	const free = courts.some((c) => {
		const cur = c.current === null ? undefined : matches.find((m) => m.id === c.current);
		return c.queue.length === 0 && (cur === undefined || cur.state === 'finished');
	});
	return free && d.nodes.some((n) => whereText(n, matches, courts, d)?.text === t.nextFreeCourt);
}

// ---- the coordinator's calls -------------------------------------------------------------

const signedIn = { signedIn: true } as const;
const bracket = (id: string, rest: string) => `/api/events/${encodeURIComponent(id)}/bracket${rest}`;

/** Shuffle the teams again; only until the first point (409 `play_started`). */
export function redrawBracket(id: string): Promise<EventDetail> {
	return accountRequest(bracket(id, '/redraw'), { ...signedIn, method: 'POST' });
}

/** A held match: replay what followed it, or keep it as played (409 `no_decision`). */
export function decideBracket(id: string, matchId: string, choice: 'replay' | 'keep'): Promise<EventDetail> {
	return accountRequest(bracket(id, `/decisions/${encodeURIComponent(matchId)}`), {
		...signedIn,
		method: 'POST',
		body: { choice }
	});
}

/** Put waiting bracket matches onto free courts. */
export function fillCourts(id: string): Promise<EventDetail> {
	return accountRequest(bracket(id, '/fill-courts'), { ...signedIn, method: 'POST' });
}

