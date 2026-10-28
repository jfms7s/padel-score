// Pure logic for the coordinator event desk and the spectator page: tabs, match grouping,
// keyboard row moves and the court tile's data.

import { teamText } from './events';
import { scoreText } from './players';
import type { EventMatch, EventStatus, EventTeam, EventViewMatch, Score, Team } from './types';
import { pointLabel } from './view';

export type DeskTab = 'live' | 'matches' | 'kind' | 'teams' | 'signups' | 'courts' | 'finish';

const TABS: Record<EventStatus, DeskTab[]> = {
	setup: ['teams', 'signups', 'courts', 'kind'],
	running: ['live', 'matches', 'kind', 'teams', 'courts', 'finish'],
	finished: ['live', 'matches', 'kind', 'teams'],
	// A cancelled event only has its sign-ups: who was in, and the cancelled notice.
	cancelled: ['signups']
};

/** The tabs an event offers in its status. The page hides an empty `kind` tab. */
export function deskTabs(status: EventStatus): DeskTab[] {
	return TABS[status];
}

export function defaultTab(status: EventStatus): DeskTab {
	if (status === 'cancelled') return 'signups';
	return status === 'setup' ? 'teams' : 'live';
}

/** The `?tab=` param if this status offers it, else the status's default tab. */
export function resolveTab(param: string | null, status: EventStatus): DeskTab {
	const tab = TABS[status].find((t) => t === param);
	return tab ?? defaultTab(status);
}

/**
 * Whether a key press is the bare `/` that jumps to the first unfinished match: not typed into
 * a field, no modifier held.
 */
export function isJumpKey(
	e: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'defaultPrevented'> & { target: EventTarget | null }
): boolean {
	if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return false;
	const target = e.target as { closest?: (selector: string) => unknown } | null;
	return !target?.closest?.('input, textarea, select, [contenteditable]:not([contenteditable="false"])');
}

export type MatchGroups<M> = { onCourt: M[]; waiting: M[]; finished: M[] };

/** Live matches and the ones a court is about to play, the waiting rest, and the finished. */
export function groupMatches<M extends Pick<EventMatch, 'state' | 'court_id' | 'ordinal'> & { id?: string }>(
	matches: M[],
	courts: { id: string; current: string | null }[]
): MatchGroups<M> {
	const current = new Set(courts.map((c) => c.current).filter((id): id is string => id !== null));
	const groups: MatchGroups<M> = { onCourt: [], waiting: [], finished: [] };
	for (const m of matches) {
		if (m.state === 'finished') groups.finished.push(m);
		else if (m.state === 'live' || (m.id !== undefined && current.has(m.id))) groups.onCourt.push(m);
		else groups.waiting.push(m);
	}
	for (const list of Object.values(groups)) list.sort((x, y) => x.ordinal - y.ordinal);
	return groups;
}

/**
 * Where the cursor goes after a row's Enter. Unsaved: stay. Saved: the next unfinished row
 * after `fromId`, wrapping round, or null when none is left.
 */
export function nextUnfinished(
	ids: string[],
	unfinished: Set<string>,
	fromId: string | null,
	saved: boolean
): string | null {
	if (!saved) return fromId;
	const start = fromId === null ? -1 : ids.indexOf(fromId);
	for (let i = 1; i <= ids.length; i++) {
		const id = ids[(start + i) % ids.length];
		if (unfinished.has(id)) return id;
	}
	return null;
}

/** Arrow-key move between rows, clamped at the ends. With no current row: the first or last. */
export function moveRow(ids: string[], current: string | null, dir: 1 | -1): string | null {
	if (ids.length === 0) return null;
	const at = current === null ? -1 : ids.indexOf(current);
	if (at === -1) return dir === 1 ? ids[0] : ids[ids.length - 1];
	return ids[Math.min(ids.length - 1, Math.max(0, at + dir))];
}

export type TileSide = { names: string; point: string; games: number; sets: number | null };
/** A finished side also has its games in each set, in order (a super tie-break's points). */
export type FinishedSide = TileSide & { setGames: number[] };

export type CourtTileData =
	| { state: 'live'; label: string; a: TileSide; b: TileSide; server: Team; serverName: string }
	| {
			state: 'finished';
			label: string;
			a: FinishedSide;
			b: FinishedSide;
			winner: Team | null;
			score: string;
			nextLabel: string | null;
	  }
	| { state: 'ready'; label: string; a: string; b: string; nextLabel: string | null }
	| { state: 'idle'; nextLabel: string | null };

function points(score: Score): [string, string] {
	const c = score.current;
	if (c === 'none') return ['', ''];
	if ('game' in c) return [pointLabel(c.game.a), pointLabel(c.game.b)];
	return [String(c.tie_break.a), String(c.tie_break.b)];
}

/**
 * What a court tile shows: the live game, the match just finished, the match on court that
 * hasn't started (waiting, or live with no score yet), or an idle court.
 */
export function courtTile(
	court: { current: string | null; queue: string[] },
	matches: (EventMatch | EventViewMatch)[],
	teams: EventTeam[]
): CourtTileData {
	const byId = (id: string | null) => (id === null ? undefined : matches.find((m) => m.id === id));
	const nextLabel = byId(court.queue[0] ?? null)?.label ?? null;
	const m = byId(court.current);
	if (!m) return { state: 'idle', nextLabel };
	const name = (id: string) => {
		const t = teams.find((x) => x.id === id);
		return t ? teamText(t) : '';
	};
	const names = [name(m.team_a), name(m.team_b)];

	if (m.state === 'live' && m.live) {
		const s = m.live;
		const timed = s.sets.length === 0;
		const p = points(s);
		const side = (i: 0 | 1): TileSide => ({
			names: names[i],
			point: p[i],
			games: s.games[i],
			sets: timed ? null : s.sets_won[i]
		});
		const serving = teams.find((x) => x.id === (s.server.team === 'a' ? m.team_a : m.team_b));
		// The serving player, or their team's name if the player can't be found.
		const serverName = serving?.players[s.server.index]?.name ?? names[s.server.team === 'a' ? 0 : 1];
		return { state: 'live', label: m.label, a: side(0), b: side(1), server: s.server.team, serverName };
	}
	if (m.state === 'finished' && m.result) {
		const r = m.result;
		const last = r.sets.length > 0 ? r.sets[r.sets.length - 1] : null;
		const games = last ? [last.a, last.b] : r.games;
		const won = (t: Team) => r.sets.filter((x) => (t === 'a' ? x.a > x.b : x.b > x.a)).length;
		// A deciding super tie-break is stored as 1-0: its points say more.
		const setGames = (i: 0 | 1) =>
			r.sets.map((x) => (x.tiebreak && x.a + x.b === 1 ? x.tiebreak[i] : i === 0 ? x.a : x.b));
		const side = (i: 0 | 1): FinishedSide => ({
			names: names[i],
			point: '',
			games: games[i],
			sets: r.timed ? null : won(i === 0 ? 'a' : 'b'),
			setGames: r.timed ? [] : setGames(i)
		});
		return {
			state: 'finished',
			label: m.label,
			a: side(0),
			b: side(1),
			winner: m.winner,
			score: scoreText(r),
			nextLabel
		};
	}
	return { state: 'ready', label: m.label, a: names[0], b: names[1], nextLabel };
}
