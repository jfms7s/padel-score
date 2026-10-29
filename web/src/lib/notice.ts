// The text of a push notification, made on the phone: the server sends a kind and data (see
// crates/server/src/push.rs `Payload`), and the service worker turns it into words here, in the
// device's own time zone. The service worker imports this file, so it may use no DOM API.

import { dayLabel, timeRange, timeText } from './dates';
import { strings } from './strings';
import type { NoticePayload } from './types';

const t = strings.notices;
const HOME = '/';

export interface NoticeText {
	title: string;
	body: string;
	url: string;
	tag: string;
}

/**
 * A same-origin path to open on tap, or the calendar. A single leading `/` and nothing that a
 * browser reads as another host (`//x`, `/\x`) or a scheme.
 */
export function safeNoticeUrl(url: unknown): string {
	if (typeof url !== 'string' || !url.startsWith('/')) return HOME;
	if (url.startsWith('//') || url.startsWith('/\\')) return HOME;
	// Control characters are stripped by URL parsers, so `/\t/evil` could become `//evil`.
	// eslint-disable-next-line no-control-regex
	if (/[\u0000-\u001f\u007f\\]/.test(url)) return HOME;
	return url;
}

export interface NoticeOptions {
	/** The device's zone; the phone's own when left out. */
	timeZone?: string;
	locale?: string;
}

/** "Wed 4 · 20:00–21:30 · Thursday padel", leaving out what the payload lacks. */
function gameBody(p: NoticePayload, opts: NoticeOptions, moved = false): string {
	// The kinds that move a game say where it is now (`new_*`), when the payload has it.
	const start = moved && p.new_start !== undefined ? p.new_start : p.start;
	const end = moved && p.new_start !== undefined ? p.new_end : p.end;
	const parts: string[] = [];
	if (start !== undefined) {
		parts.push(dayLabel(start, opts.timeZone, opts.locale));
		if (end !== undefined) parts.push(timeRange(start, end, opts.timeZone, opts.locale));
	}
	if (p.title) parts.push(p.title);
	return parts.join(' · ');
}

/** "Wed 4 20:00 · Sunday cup": when an event starts and its name, leaving out what is missing. */
function eventBody(p: NoticePayload, opts: NoticeOptions): string {
	const parts: string[] = [];
	if (p.start !== undefined) {
		parts.push(`${dayLabel(p.start, opts.timeZone, opts.locale)} ${timeText(p.start, opts.timeZone, opts.locale)}`);
	}
	if (p.title) parts.push(p.title);
	return parts.join(' · ');
}

/** "Wed 4 20:00 · Ana": when a scheduled match starts and who organises it. */
function matchBody(p: NoticePayload, opts: NoticeOptions): string {
	const parts: string[] = [];
	if (p.start !== undefined) {
		parts.push(`${dayLabel(p.start, opts.timeZone, opts.locale)} ${timeText(p.start, opts.timeZone, opts.locale)}`);
	}
	if (p.name) parts.push(p.name);
	return parts.join(' · ');
}

/** Words for a push payload; `opts` sets the zone and locale instants are printed in. */
export function noticeText(p: NoticePayload, opts: NoticeOptions = {}): NoticeText {
	const url = safeNoticeUrl(p.url);
	// The start keeps two notices from one board from replacing each other on the phone.
	const tag = `${String(p.kind)}:${url}:${p.start ?? ''}`;
	const made = (title: string, body: string): NoticeText => ({ title, body, url, tag });
	switch (p.kind) {
		case 'test':
			return made(t.test.title, t.test.body);
		case 'game_formed':
			return made(t.gameFormed, gameBody(p, opts));
		case 'moved_in':
			return made(t.movedIn, gameBody(p, opts));
		case 'player_left':
			return made(t.playerLeft(p.name), gameBody(p, opts));
		case 'board_deleted':
			return made(t.boardDeleted, gameBody(p, opts));
		case 'place_given':
			return made(t.placeGiven(p.name), gameBody(p, opts, true));
		case 'swap_asked':
			return made(t.swapAsked(p.name), gameBody(p, opts));
		case 'swap_answered':
			return made(p.accepted === false ? t.swapAnswered.declined : t.swapAnswered.accepted, gameBody(p, opts));
		case 'time_asked':
			return made(t.timeAsked(p.name), gameBody(p, opts, true));
		case 'time_answered':
			return made(p.accepted === false ? t.timeAnswered.declined : t.timeAnswered.accepted, gameBody(p, opts));
		case 'time_changed':
			return made(t.timeChanged, gameBody(p, opts, true));
		case 'event_cancelled':
			return made(t.eventCancelled, eventBody(p, opts));
		case 'event_changed':
			return made(t.eventChanged, eventBody(p, opts));
		case 'team_complete':
			return made(t.teamComplete, eventBody(p, opts));
		case 'moved_up':
			return made(t.movedUp, eventBody(p, opts));
		case 'moved_down':
			return made(t.movedDown, eventBody(p, opts));
		case 'partner_left':
			return made(t.partnerLeft(p.name), eventBody(p, opts));
		case 'no_place':
			return made(t.noPlace, eventBody(p, opts));
		case 'place_opened':
			return made(t.placeOpened, matchBody(p, opts));
		case 'match_changed':
			return made(t.matchChanged, matchBody(p, opts));
		case 'match_cancelled':
			return made(t.matchCancelled, matchBody(p, opts));
		case 'removed_from_match':
			return made(t.removedFromMatch, matchBody(p, opts));
		case 'match_short':
			return made(t.matchShort, matchBody(p, opts));
		case 'short_game':
			return made(t.shortGame, gameBody(p, opts));
		case 'event_soon':
			return made(t.eventSoon, eventBody(p, opts));
		default:
			return made(t.appName, t.fallback);
	}
}
