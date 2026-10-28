// The text of a push notification, made on the phone: the server sends a kind and data (see
// crates/server/src/push.rs `Payload`), and the service worker turns it into words here, in the
// device's own time zone. The service worker imports this file, so it may use no DOM API.

import { dayLabel, timeRange } from './dates';
import { strings } from './strings';
import type { NoticePayload } from './types';

const t = strings.notices;
const HOME = '/calendar';

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
function gameBody(p: NoticePayload, opts: NoticeOptions): string {
	const parts: string[] = [];
	if (p.start !== undefined) {
		parts.push(dayLabel(p.start, opts.timeZone, opts.locale));
		if (p.end !== undefined) parts.push(timeRange(p.start, p.end, opts.timeZone, opts.locale));
	}
	if (p.title) parts.push(p.title);
	return parts.join(' · ');
}

/** Words for a push payload; `opts` sets the zone and locale instants are printed in. */
export function noticeText(p: NoticePayload, opts: NoticeOptions = {}): NoticeText {
	const url = safeNoticeUrl(p.url);
	const tag = `${String(p.kind)}:${url}`;
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
		default:
			return made(t.appName, t.fallback);
	}
}
