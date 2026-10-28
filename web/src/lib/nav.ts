// The wide-screen app rail: its destinations, and which screens go without app chrome.

import { strings } from './strings';

export type RailItem = { href: string; label: string; current: boolean };

const ITEMS = [
	{ href: '/', label: strings.nav.home },
	{ href: '/calendar', label: strings.nav.calendar },
	{ href: '/events', label: strings.nav.events },
	{ href: '/players', label: strings.nav.players },
	{ href: '/history', label: strings.nav.history },
	{ href: '/profile', label: strings.nav.results }
] as const;

/** The rail's items; one is current when the path is it or under it (Home only on `/`). */
export function railItems(pathname: string): RailItem[] {
	return ITEMS.map(({ href, label }) => ({
		href,
		label,
		current: href === '/' ? pathname === '/' : pathname === href || pathname.startsWith(`${href}/`)
	}));
}

/** The fence, court and spectator screens are full-screen and anonymous-friendly: no app chrome. */
export function isBare(pathname: string): boolean {
	return /^\/(s|v|c|cv)\//.test(pathname);
}

/** Up to two initials for the account circle: first letters of the first and last word. */
export function initials(name: string): string {
	const words = name.trim().split(/\s+/).filter(Boolean);
	if (words.length === 0) return '?';
	const first = words[0]!;
	const last = words.length > 1 ? words[words.length - 1]! : '';
	return (Array.from(first)[0]! + (Array.from(last)[0] ?? '')).toUpperCase();
}
