// Home's pure helpers: the live matches, the week strip and the agenda lines. Home draws from
// these; the calendar fetch is calendar.ts and the dates are dates.ts.

import { addDays, localDate, longDate, timeText } from './dates';
import { itemStart } from './calendar';
import { scorerPath, type RecentMatch } from './setup';
import { strings } from './strings';
import type { CalendarItem, MyMatch } from './types';

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

/** The agenda's second line, parts joined with " · ". */
export function agendaNote(item: CalendarItem): string {
	const t = strings.home;
	let parts: (string | null)[];
	switch (item.kind) {
		case 'game':
			parts = [t.game, item.board_place, item.board_title, item.state === 'short' ? t.short : null];
			break;
		case 'event':
			parts = [
				item.place,
				item.max_teams === null ? t.teams(item.teams) : strings.calendar.teamsOpen(item.teams, item.max_teams),
				item.role === 'line' ? t.inLine : null
			];
			break;
		case 'match': {
			const open = item.players.filter((p) => p === null).length;
			parts = [item.place, open > 0 ? t.open(open) : null];
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

/** The hours (0-23, in the zone) the week grid needs: every hour an item starting that week touches. */
export function shownHours(items: CalendarItem[], first: string, timeZone: string): number[] {
	const last = addDays(first, 6);
	const hours = new Set<number>();
	for (const item of items) {
		const start = itemStart(item);
		const date = localDate(start, timeZone);
		if (date < first || date > last) continue;
		// Back to the top of the hour it starts in, then an hour at a time until it ends.
		const minutes = Number(timeText(start, timeZone).slice(3, 5));
		for (let t = start - minutes * 60; t < itemEnd(item); t += 3600) {
			hours.add(Number(timeText(t, timeZone).slice(0, 2)));
		}
	}
	return hours.size === 0 ? EVENING : [...hours].sort((a, b) => a - b);
}
