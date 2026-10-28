// The calendar: one list over the games, events and matches a user plays or organises, what waits
// for their answer, and what is open to join. The server is crates/server/src/calendar_api.rs;
// this file reads it and groups it by day in the device's own zone.

import { accountRequest } from './api';
import { addDays, browserTimeZone, localDate, localMidnight, timeRange, timeText } from './dates';
import type { StorageLike } from './setup';
import { strings } from './strings';
import type { Calendar, CalendarItem, OpenItem } from './types';

/** Set once someone says "Not now" to the notifications card. */
export const PUSH_PROMPT_KEY = 'padel-score:push-prompt-dismissed';

/** An hour after its start an event or a game still counts as "now" for Next up (spec: P135). */
const STARTED_GRACE_SECS = 3600;

/** The calendar from `from` on; the server reads back at most two days before now. */
export function getCalendar(from: number): Promise<Calendar> {
	return accountRequest(`/api/me/calendar?from=${Math.floor(from)}`, { signedIn: true });
}

/** The instant today starts in the zone (the device's when left out). */
export function startOfToday(timeZone?: string, now = Date.now() / 1000): number {
	const zone = timeZone ?? browserTimeZone();
	return localMidnight(localDate(now, zone), zone);
}

/** When an item starts. */
export function itemStart(item: CalendarItem | OpenItem): number {
	return item.kind === 'game' ? item.start : item.starts_at;
}

/** The items grouped by the local day they start on, in the order given. */
export function byDay(items: CalendarItem[], timeZone?: string): { date: string; items: CalendarItem[] }[] {
	const days: { date: string; items: CalendarItem[] }[] = [];
	for (const item of items) {
		const date = localDate(itemStart(item), timeZone);
		const last = days[days.length - 1];
		if (last?.date === date) last.items.push(item);
		else days.push({ date, items: [item] });
	}
	return days;
}

/** What to call an item: the board, the event, or the match with its place. */
export function itemTitle(item: CalendarItem): string {
	switch (item.kind) {
		case 'game':
			return item.board_title;
		case 'event':
			return item.name;
		case 'match':
			return item.place ? `${strings.calendar.match} · ${item.place}` : strings.calendar.match;
	}
}

/** "20:00–21:30" for a game, "20:00" for an event or a match (their end is not planned). */
export function itemTime(item: CalendarItem, timeZone?: string): string {
	return item.kind === 'game' ? timeRange(item.start, item.end, timeZone) : timeText(item.starts_at, timeZone);
}

/** The line under the title: the game's state, or the viewer's part in an event or a match. */
export function itemNote(item: CalendarItem): string {
	const t = strings.calendar;
	switch (item.kind) {
		case 'game':
			return strings.boards.gameState[item.state];
		case 'event': {
			if (item.status === 'cancelled') return strings.events.status.cancelled;
			if (item.role === 'line' && item.place_in_line !== null) return t.inLine(item.place_in_line);
			return t.role[item.role];
		}
		case 'match': {
			// A full match says what the viewer is in it; the other states say what became of it.
			if (item.state !== 'open' && item.state !== 'full') return t.matchState[item.state];
			return t.role[item.role];
		}
	}
}

/** The line under the note: who plays a game, or the place and team of an event. Empty if none. */
export function itemDetail(item: CalendarItem): string {
	switch (item.kind) {
		case 'game':
			return item.players.join(', ');
		case 'event':
			return [item.place, item.team?.join(' & ')].filter(Boolean).join(' · ');
		case 'match':
			return '';
	}
}

const enc = encodeURIComponent;

/** Where an item opens: its board with the game's sheet, the organiser's event, or a join link. */
export function itemHref(item: CalendarItem | OpenItem): string {
	switch (item.kind) {
		case 'game':
			return `/b/${enc(item.board_token)}?game=${enc(item.game_id)}`;
		case 'event':
			if ('event_id' in item && item.event_id) return `/events/${enc(item.event_id)}`;
			return item.join_token ? `/j/${enc(item.join_token)}` : '/calendar';
		case 'match':
			return `/j/${enc(item.join_token)}`;
	}
}

/**
 * The first item worth showing as "Next up": a full or short game that has not ended, an event
 * in setup or running that started less than an hour ago or later, or an open or full match.
 */
export function nextUp(cal: Calendar, now: number): CalendarItem | null {
	return (
		cal.items.find((item) => {
			switch (item.kind) {
				case 'game':
					return (item.state === 'full' || item.state === 'short') && item.end > now;
				case 'event':
					return (item.status === 'setup' || item.status === 'running') && item.starts_at >= now - STARTED_GRACE_SECS;
				case 'match':
					return item.state === 'open' || item.state === 'full';
			}
		}) ?? null
	);
}

/** The Monday of the week a day ("2026-11-04") falls in. */
export function weekStart(date: string): string {
	const weekday = new Date(`${date}T00:00:00Z`).getUTCDay(); // 0 = Sunday
	return addDays(date, -((weekday + 6) % 7));
}

/** `date` moved by `by` whole weeks (negative for earlier). */
export function shiftWeek(date: string, by: number): string {
	return addDays(date, by * 7);
}

/** True when this device already said "Not now". A store that throws counts as not dismissed. */
export function pushPromptDismissed(storage: StorageLike): boolean {
	try {
		return storage.getItem(PUSH_PROMPT_KEY) !== null;
	} catch {
		return false;
	}
}

export function dismissPushPrompt(storage: StorageLike): void {
	try {
		storage.setItem(PUSH_PROMPT_KEY, '1');
	} catch {
		// Without storage the card comes back next visit; nothing worse.
	}
}
