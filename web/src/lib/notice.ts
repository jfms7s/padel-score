// The text of a push notification, made on the phone: the server sends a kind and data (see
// crates/server/src/push.rs `Payload`), and the service worker turns it into words here, in the
// device's own time zone. The service worker imports this file, so it may use no DOM API.

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

/** `opts` is for the kinds that print instants; the ones known so far print none. */
export function noticeText(p: NoticePayload, _opts: { timeZone?: string; locale?: string } = {}): NoticeText {
	const url = safeNoticeUrl(p.url);
	const tag = `${String(p.kind)}:${url}`;
	if (p.kind === 'test') return { title: t.test.title, body: t.test.body, url, tag };
	return { title: t.appName, body: t.fallback, url, tag };
}
