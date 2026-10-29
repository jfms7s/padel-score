// The calendar: one list over the games, events and matches a user plays or organises, what waits
// for their answer, and what is open to join. The server is crates/server/src/calendar_api.rs;
// this file reads it and groups it by day in the device's own zone.

import { accountRequest } from './api';
import { addDays, browserTimeZone, localDate, localMidnight, timeRange, timeText } from './dates';
import type { StorageLike } from './setup';
import type { Calendar, CalendarItem, OpenItem } from './types';

/** Set once someone says "Not now" to the notifications card. */
export const PUSH_PROMPT_KEY = 'padel-score:push-prompt-dismissed';

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

/** "20:00–21:30" for a game, "20:00" for an event or a match (their end is not planned). */
export function itemTime(item: CalendarItem, timeZone?: string): string {
	return item.kind === 'game' ? timeRange(item.start, item.end, timeZone) : timeText(item.starts_at, timeZone);
}

const enc = encodeURIComponent;

/** Where an item opens: its board with the game's sheet, the organiser's event, or a join link. */
export function itemHref(item: CalendarItem | OpenItem): string {
	switch (item.kind) {
		case 'game':
			return `/b/${enc(item.board_token)}?game=${enc(item.game_id)}`;
		case 'event':
			if ('event_id' in item && item.event_id) return `/events/${enc(item.event_id)}`;
			return item.join_token ? `/j/${enc(item.join_token)}` : '/';
		case 'match':
			return `/j/${enc(item.join_token)}`;
	}
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
