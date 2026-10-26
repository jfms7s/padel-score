// The fence screen's pure helpers: which view to show, where the serve ball goes, the change
// of ends line, the layout choice and the share links.

import { strings } from './strings';
import type { FenceModel, TeamModel } from './view';
import type { Event, Format, MatchView, ReplayResult, Start, Team } from './types';
import type { StorageLike } from './setup';
import type { Pending } from './queue';

/**
 * The view to show next. Revisions only grow, so an incoming view older than the one shown is
 * stale (a late SSE frame, a replayed response) and is ignored: the fence never steps back.
 */
export function latestView(current: MatchView | null, incoming: MatchView): MatchView {
	if (current === null || incoming.revision > current.revision) return incoming;
	return current;
}

export interface BallPosition {
	team: Team;
	/** From the server's own point of view; `centre` when the receiver picks the side. */
	side: 'right' | 'left' | 'centre';
}

/** Where the serve ball sits: behind the server's baseline, on the side they serve from. */
export function ballPosition(model: FenceModel): BallPosition | null {
	if (model.serveSide === null) return null;
	const team: Team = model.a.serving ? 'a' : 'b';
	const side = model.serveSide === 'receiver_chooses' ? 'centre' : model.serveSide;
	return { team, side };
}

/** "Change ends. Ana / Rui lead 5–3 in set 2." */
export function changeEndsLine(model: FenceModel): string {
	const c = strings.changeEnds;
	const { a, b } = model;
	if (model.inTieBreak) {
		const pa = Number(a.points);
		const pb = Number(b.points);
		if (pa === pb) return c.tieBreakLevel(pa, pb);
		return pa > pb ? c.tieBreakLead(a.name, pa, pb) : c.tieBreakLead(b.name, pb, pa);
	}
	const set = a.sets.length;
	if (set === 0) {
		// Timed: games across the whole match.
		if (a.games === b.games) return c.level(a.games, b.games);
		return a.games > b.games ? c.lead(a.name, a.games, b.games) : c.lead(b.name, b.games, a.games);
	}
	if (a.games === 0 && b.games === 0 && set > 1) {
		const ga = a.sets[set - 2] ?? 0;
		const gb = b.sets[set - 2] ?? 0;
		return ga > gb ? c.tookSet(a.name, set - 1, ga, gb) : c.tookSet(b.name, set - 1, gb, ga);
	}
	if (a.games === b.games) return c.levelInSet(a.games, b.games, set);
	return a.games > b.games ? c.leadInSet(a.name, a.games, b.games, set) : c.leadInSet(b.name, b.games, a.games, set);
}

/** The finished sets as "7–6 (5), 6–3": games, a tie-break loser's points, a super tie-break's points. */
export function setsSummary(model: FenceModel): string {
	const { a, b } = model;
	return a.setDetails
		.map((da, i) => {
			const db = b.setDetails[i];
			if (!db) return '';
			if (da.superTieBreak && da.tiebreak !== null && db.tiebreak !== null) return `${da.tiebreak}–${db.tiebreak}`;
			const loser = da.tiebreak ?? db.tiebreak;
			return loser === null ? `${da.games}–${db.games}` : `${da.games}–${db.games} (${loser})`;
		})
		.filter((s) => s !== '')
		.join(', ');
}

/** The team's own label on a tap button: "Point Ana". */
export function pointButtonLabel(team: TeamModel): string {
	return strings.fence.pointShort(team.name.split(' / ')[0] ?? team.name);
}

// ---- layout ------------------------------------------------------------------------------

export type Layout = 'auto' | 'court' | 'board';
export const LAYOUT_KEY = 'padel-score:layout';
const LAYOUTS: Layout[] = ['auto', 'court', 'board'];

export function loadLayout(storage: StorageLike): Layout {
	try {
		const raw = storage.getItem(LAYOUT_KEY);
		return LAYOUTS.find((l) => l === raw) ?? 'auto';
	} catch {
		return 'auto';
	}
}

export function saveLayout(storage: StorageLike, layout: Layout): void {
	try {
		storage.setItem(LAYOUT_KEY, layout);
	} catch {
		// A convenience.
	}
}

export function nextLayout(layout: Layout): Layout {
	return LAYOUTS[(LAYOUTS.indexOf(layout) + 1) % LAYOUTS.length] ?? 'auto';
}

// ---- sharing -----------------------------------------------------------------------------

/** The spectator token from the scorer page's fragment (`#v=...`). */
export function spectatorFromHash(hash: string): string | null {
	const v = new URLSearchParams(hash.replace(/^#/, '')).get('v');
	return v ? v : null;
}

/** Absolute links for the share sheet. The scoring link carries the viewing token along. */
export function shareLinks(
	origin: string,
	scorer: string,
	spectator: string | null
): { scoring: string; viewing: string | null } {
	const enc = encodeURIComponent;
	return {
		scoring: `${origin}/s/${enc(scorer)}${spectator ? `#v=${enc(spectator)}` : ''}`,
		viewing: spectator ? `${origin}/v/${enc(spectator)}` : null
	};
}

// ---- optimistic taps ---------------------------------------------------------------------

/**
 * The pending points to lay over `view`. A tap not yet sent can't be in any server view, so it
 * always shows. A sent one (the head, in flight) may already be in a view the stream delivered
 * before its HTTP answer: it shows only while the view is no newer than the one it was made on.
 */
export function overlayTaps(view: MatchView, pending: Pending[]): Team[] {
	return pending.flatMap((p) =>
		p.intent.kind === 'point' && (!p.sent || p.base >= view.revision) ? [p.intent.team] : []
	);
}
