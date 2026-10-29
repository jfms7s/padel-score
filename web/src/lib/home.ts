// Home's pure helpers: the live matches, the week strip and the agenda lines. Home draws from
// these; the calendar fetch is calendar.ts and the dates are dates.ts.

import { addDays, localDate, longDate, timeText } from './dates';
import { itemStart } from './calendar';
import { liveScoreText } from './events';
import { scorerPath, type RecentMatch } from './setup';
import { strings } from './strings';
import type { CalendarItem, MyMatch, OpenItem } from './types';

/** A live match with no point for this long is "older": offered for tidying, not for resuming. */
export const STALE_SECS = 86_400;

/** How long an event or a match is drawn for, as nothing plans their end. */
const MATCH_SECS = 5400;
const EVENT_SECS = 7200;

/** The hours a week with nothing in it shows. */
const EVENING = [18, 19, 20, 21];

export type Kind = CalendarItem['kind'];

export interface LiveRow {
	label: string;
	href: string;
	scorerToken: string;
	/** The score as it stands; null when this device only knows the match's name. */
	score: string | null;
	when: string;
	deletable: boolean;
}

/** The live matches this session can open, in server order, split at a day of silence. */
export function liveNow(mine: MyMatch[], now: number, timeZone?: string): { fresh: LiveRow[]; older: LiveRow[] } {
	const fresh: LiveRow[] = [];
	const older: LiveRow[] = [];
	for (const m of mine) {
		if (typeof m.scorer_token !== 'string') continue;
		const row: LiveRow = {
			label: m.label,
			href: scorerPath(m.scorer_token, m.spectator_token),
			scorerToken: m.scorer_token,
			score: liveScoreText(m.view.score),
			when: longDate(m.last_activity_at, timeZone),
			deletable: !m.context
		};
		(m.last_activity_at >= now - STALE_SECS ? fresh : older).push(row);
	}
	return { fresh, older };
}

/** The matches started on this device, for when the account can't be reached. */
export function offlineRows(recent: RecentMatch[], timeZone?: string): LiveRow[] {
	return recent.map((r) => ({
		label: r.label,
		href: scorerPath(r.scorer_token, r.spectator_token),
		scorerToken: r.scorer_token,
		score: null,
		when: longDate(r.created_at / 1000, timeZone),
		deletable: true
	}));
}

/** The seven days from `first` (a Monday), each with the kinds of the items starting on it. */
export function weekPips(items: CalendarItem[], first: string, timeZone: string): { date: string; kinds: Kind[] }[] {
	return Array.from({ length: 7 }, (_, i) => {
		const date = addDays(first, i);
		const kinds = items.filter((it) => localDate(itemStart(it), timeZone) === date).map((it) => it.kind);
		return { date, kinds };
	});
}

/** The agenda's title: who plays a game, the event's name, or whose match it is. */
export function agendaTitle(item: CalendarItem): string {
	switch (item.kind) {
		case 'game':
			return strings.home.withYou(item.players.filter((_, i) => i !== item.me).join(', '));
		case 'event':
			return item.name;
		case 'match':
			if (item.role !== 'organiser') return strings.home.matchBy(item.owner_name);
			return item.place ? `${strings.calendar.match} · ${item.place}` : strings.calendar.match;
	}
}

/** Whether the server sent this item as cancelled (it does so on purpose, so it can be marked). */
export function isCancelled(item: CalendarItem): boolean {
	switch (item.kind) {
		case 'game':
			return false;
		case 'event':
			return item.status === 'cancelled';
		case 'match':
			return item.state === 'cancelled';
	}
}

/** The agenda's second line, parts joined with " · ". A cancelled or started item leads with that. */
export function agendaNote(item: CalendarItem): string {
	const t = strings.home;
	const states = strings.calendar.matchState;
	let parts: (string | null)[];
	switch (item.kind) {
		case 'game':
			parts = [t.game, item.board_place, item.board_title, item.state === 'short' ? t.short : null];
			break;
		case 'event':
			parts = [
				item.status === 'cancelled' ? states.cancelled : null,
				item.place,
				item.max_teams === null ? t.teams(item.teams) : strings.calendar.teamsOpen(item.teams, item.max_teams),
				item.role === 'line' ? t.inLine : null
			];
			break;
		case 'match': {
			const open = item.players.filter((p) => p === null).length;
			const lead = item.state === 'cancelled' ? states.cancelled : item.state === 'started' ? states.started : null;
			parts = [lead, item.place, open > 0 ? t.open(open) : null];
			break;
		}
	}
	return parts.filter(Boolean).join(' · ');
}

/** When an item ends: a game knows, a match and an event are drawn for a usual length. */
export function itemEnd(item: CalendarItem): number {
	switch (item.kind) {
		case 'game':
			return item.end;
		case 'match':
			return item.starts_at + MATCH_SECS;
		case 'event':
			return item.starts_at + EVENT_SECS;
	}
}

/** When an open-to-join item ends: drawn for a usual length, as nothing plans its end. */
export function openEnd(item: OpenItem): number {
	return item.starts_at + (item.kind === 'match' ? MATCH_SECS : EVENT_SECS);
}

/** What selects an item on the week grid (and sits in `?item=`). */
export function itemKey(item: CalendarItem): string {
	switch (item.kind) {
		case 'game':
			return `game:${item.game_id}`;
		case 'event':
			return `event:${item.event_id ?? item.join_token}`;
		case 'match':
			return `match:${item.join_token}`;
	}
}

/** Side-by-side lanes for blocks that overlap in time, in the order given. */
export function laneLayout(blocks: { start: number; end: number }[]): { lane: number; lanes: number }[] {
	const out: { lane: number; lanes: number }[] = blocks.map(() => ({ lane: 0, lanes: 1 }));
	const order = blocks.map((_, i) => i).sort((a, b) => (blocks[a]?.start ?? 0) - (blocks[b]?.start ?? 0));
	// A cluster is a run of blocks that chain together by overlap; its lanes share the width.
	let cluster: number[] = [];
	let clusterEnd = -Infinity;
	let laneEnds: number[] = [];
	const close = () => {
		for (const i of cluster) {
			const o = out[i];
			if (o) o.lanes = laneEnds.length;
		}
		cluster = [];
		laneEnds = [];
	};
	for (const i of order) {
		const b = blocks[i];
		const o = out[i];
		if (!b || !o) continue;
		if (b.start >= clusterEnd) close();
		let lane = laneEnds.findIndex((end) => end <= b.start);
		if (lane === -1) lane = laneEnds.length;
		laneEnds[lane] = b.end;
		o.lane = lane;
		cluster.push(i);
		clusterEnd = cluster.length === 1 ? b.end : Math.max(clusterEnd, b.end);
	}
	close();
	return out;
}

/**
 * The hours (0-23, in the zone) the week grid needs: every hour an item or an open-to-join item
 * starting that week touches.
 */
export function shownHours(items: CalendarItem[], first: string, timeZone: string, open: OpenItem[] = []): number[] {
	const last = addDays(first, 6);
	const hours = new Set<number>();
	const spans = [
		...items.map((it) => ({ start: itemStart(it), end: itemEnd(it) })),
		...open.map((o) => ({ start: o.starts_at, end: openEnd(o) }))
	];
	for (const { start, end } of spans) {
		const date = localDate(start, timeZone);
		if (date < first || date > last) continue;
		// Back to the top of the hour it starts in, then an hour at a time until it ends.
		const minutes = Number(timeText(start, timeZone).slice(3, 5));
		for (let t = start - minutes * 60; t < end; t += 3600) {
			hours.add(Number(timeText(t, timeZone).slice(0, 2)));
		}
	}
	return hours.size === 0 ? EVENING : [...hours].sort((a, b) => a - b);
}
