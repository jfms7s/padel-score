// The week grid's maths, apart from the screen: half-hour cells, ranges from two taps, who is
// free when, and how to turn the grid. A cell is the instant its half hour starts.

import type { Day } from './dates';
import { timeText } from './dates';
import type { BoardView, GameOut, MemberRef, PieceOut } from './types';

export const SLOT_SECS = 1800;
/** Below this width the grid turns: days become columns and hours run down. */
const NARROW_PX = 768;

export interface Range {
	start: number;
	end: number;
}

/** The half-hour starts of a local day: 48, or 46 or 50 where the clocks change. With an
 *  `anchor` (the board's start) the cells sit on the grid that instant sets. */
export function slotsOfDay(day: { start: number; end: number }, anchor?: number): number[] {
	const out: number[] = [];
	// The server takes a time only on the board's own half-hour grid, which a viewer in a zone
	// that is not a whole half hour from the board's starts the day part way through a clock half hour.
	const offset = anchor === undefined ? 0 : (((anchor - day.start) % SLOT_SECS) + SLOT_SECS) % SLOT_SECS;
	for (let t = day.start + offset; t < day.end; t += SLOT_SECS) out.push(t);
	return out;
}

/** A cell with the row (or column) of its clock time, 0 at 00:00. */
export interface Cell {
	t: number;
	row: number;
}

const CLOCK_ROWS = 48;

/**
 * A day's cells on clock rows, so 04:00 is the same row on every day. Where the clocks go back
 * a time is read twice: its second reading goes on an extra row after the 48 clock rows. Where
 * they go forward the skipped rows have no cell.
 */
export function dayCells(day: Pick<Day, 'start' | 'end'>, timeZone: string, anchor?: number): Cell[] {
	const seen = new Set<number>();
	let extra = CLOCK_ROWS;
	return slotsOfDay(day, anchor).map((t) => {
		const [h, m] = timeText(t, timeZone).split(':').map(Number);
		const clock = (h ?? 0) * 2 + ((m ?? 0) >= 30 ? 1 : 0);
		if (!seen.has(clock)) {
			seen.add(clock);
			return { t, row: clock };
		}
		return { t, row: extra++ };
	});
}

/**
 * The row an arrow key moves to: the nearest row from `row` in the direction `dir` (1 later, -1
 * earlier) that the day has a cell on, or null at the end of the day. A day where the clocks
 * went forward has no cells on the skipped rows, so the step jumps over them.
 */
export function steppedRow(rows: ReadonlySet<number>, row: number, dir: 1 | -1): number | null {
	if (rows.size === 0) return null;
	const last = Math.max(...rows);
	for (let r = row + dir; r >= 0 && r <= last; r += dir) if (rows.has(r)) return r;
	return null;
}

/** The range two cells make, whichever was tapped first: it ends after the later cell. */
export function rangeFromCells(a: number, b: number): Range {
	return { start: Math.min(a, b), end: Math.max(a, b) + SLOT_SECS };
}

const covers = (p: { start: number; end: number }, t: number) => p.start <= t && t < p.end;

type Busy = Pick<GameOut, 'start' | 'end' | 'players'>;

/** Is the member in a game that overlaps the half hour starting `t`? */
function inGame(games: Busy[], userId: string, t: number): boolean {
	return games.some((g) => g.start < t + SLOT_SECS && t < g.end && g.players.some((p) => p.user_id === userId));
}

/** How many people other than `me` are free at the half hour starting `t`: covered by a piece
 *  and not in a game then (the same people `freeAt` lists). */
export function othersFreeCount(pieces: PieceOut[], me: string | null, t: number, games: Busy[] = []): number {
	const who = new Set<string>();
	for (const p of pieces) if (p.user_id !== me && covers(p, t) && !inGame(games, p.user_id, t)) who.add(p.user_id);
	return who.size;
}

/** `othersFreeCount` for every half hour at once: half hour start -> count (absent means 0). */
export function heatMap(pieces: PieceOut[], me: string | null, games: Busy[] = []): Map<number, number> {
	const people = new Map<number, Set<string>>();
	for (const p of pieces) {
		if (p.user_id === me) continue;
		// Pieces sit on the board's grid, which is not epoch-aligned in a :15 or :45 zone.
		for (let t = p.start; t < p.end; t += SLOT_SECS) {
			if (inGame(games, p.user_id, t)) continue;
			const set = people.get(t) ?? new Set<string>();
			set.add(p.user_id);
			people.set(t, set);
		}
	}
	return new Map([...people].map(([t, set]) => [t, set.size]));
}

export type Shade = 'none' | 'some' | 'game';

/** 0 others is nothing, 1-2 some, 3 or more is enough for a game. */
export function shade(count: number): Shade {
	if (count >= 3) return 'game';
	return count >= 1 ? 'some' : 'none';
}

/**
 * Who is free at the half hour starting `t`, first in line first: by the creation time of the
 * piece that covers it, then when they joined, then user id. Someone already in a game then is
 * not free.
 */
export function freeAt(view: BoardView, t: number): MemberRef[] {
	const busy = new Set(
		view.games.filter((g) => g.start < t + SLOT_SECS && t < g.end).flatMap((g) => g.players.map((p) => p.user_id))
	);
	const since = new Map<string, number>();
	for (const p of view.pieces) {
		if (covers(p, t)) since.set(p.user_id, Math.min(since.get(p.user_id) ?? Infinity, p.created_at));
	}
	return view.members
		.filter((m) => since.has(m.user_id) && !busy.has(m.user_id))
		.sort(
			(a, b) =>
				(since.get(a.user_id) ?? 0) - (since.get(b.user_id) ?? 0) ||
				a.joined_at - b.joined_at ||
				(a.user_id < b.user_id ? -1 : a.user_id > b.user_id ? 1 : 0)
		)
		.map(({ user_id, name, guest }) => ({ user_id, name, guest }));
}

/** One member's pieces with touching or overlapping ones joined, in time order. */
export function mergedRanges(pieces: PieceOut[], userId: string): Range[] {
	const mine = pieces.filter((p) => p.user_id === userId).sort((a, b) => a.start - b.start || a.end - b.end);
	const out: Range[] = [];
	for (const p of mine) {
		const last = out[out.length - 1];
		if (last && p.start <= last.end) last.end = Math.max(last.end, p.end);
		else out.push({ start: p.start, end: p.end });
	}
	return out;
}

/** Days across and hours down on a phone; days down and hours across from tablet width. */
export function orientation(width: number): 'rows' | 'columns' {
	return width < NARROW_PX ? 'columns' : 'rows';
}

export interface GameLane {
	id: string;
	/** 0-based lane across the column (or down the row) the game is drawn in. */
	lane: number;
	/** How many lanes the games it overlaps (directly or through others) share. */
	lanes: number;
}

/**
 * Where to draw games that overlap in time so that each keeps its own outline and button: side
 * by side, the column split among them. The viewer's own games come first, then earlier starts,
 * then the order given (formed order). A game takes the first lane free of the games it
 * overlaps; the games linked by overlap share one lane count.
 */
export function layoutGames(
	games: { id: string; start: number; end: number; players: { user_id: string }[] }[],
	me: string | null
): GameLane[] {
	const ordered = games
		.map((g, i) => ({ g, i, mine: me !== null && g.players.some((p) => p.user_id === me) }))
		.sort((a, b) => Number(b.mine) - Number(a.mine) || a.g.start - b.g.start || a.i - b.i);
	const lane = new Map<string, number>();
	for (const { g } of ordered) {
		const taken = new Set(
			ordered
				.filter(({ g: o }) => o.id !== g.id && lane.has(o.id) && o.start < g.end && g.start < o.end)
				.map(({ g: o }) => lane.get(o.id))
		);
		let n = 0;
		while (taken.has(n)) n++;
		lane.set(g.id, n);
	}
	// Group by overlap (a connected cluster) so a cluster's games share one width.
	const cluster = new Map<string, number>();
	let next = 0;
	for (const { g } of ordered) {
		if (cluster.has(g.id)) continue;
		const id = next++;
		const stack = [g];
		while (stack.length > 0) {
			const cur = stack.pop();
			if (!cur || cluster.has(cur.id)) continue;
			cluster.set(cur.id, id);
			for (const { g: o } of ordered) if (!cluster.has(o.id) && o.start < cur.end && cur.start < o.end) stack.push(o);
		}
	}
	const lanesOf = new Map<number, number>();
	for (const { g } of ordered) {
		const c = cluster.get(g.id) ?? 0;
		lanesOf.set(c, Math.max(lanesOf.get(c) ?? 0, (lane.get(g.id) ?? 0) + 1));
	}
	return ordered.map(({ g }) => ({
		id: g.id,
		lane: lane.get(g.id) ?? 0,
		lanes: lanesOf.get(cluster.get(g.id) ?? 0) ?? 1
	}));
}
