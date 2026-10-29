// The app's navigation model: its five places, how much chrome a screen gets, and back links.

import { strings } from './strings';

export type NavKey = 'home' | 'organise' | 'new' | 'results' | 'people';
export type NavItem = { key: NavKey; href: string; label: string; current: boolean };
export type Chrome = 'bare' | 'standalone' | 'auth' | 'app';
export type Parent = { href: string; label: string };

/** Where each place leads; later phases repoint these. */
export const NAV_TARGETS: Record<NavKey, string> = {
	home: '/',
	organise: '/events',
	new: '/new',
	results: '/profile',
	people: '/players'
};

const ORDER: NavKey[] = ['home', 'organise', 'new', 'results', 'people'];

/** The path prefixes that make a place current. Home is exact; New is never current. */
const SECTIONS: Record<Exclude<NavKey, 'home' | 'new'>, string[]> = {
	organise: ['/events', '/organise', '/new/event'],
	results: ['/results', '/profile', '/history'],
	people: ['/players']
};

const under = (pathname: string, prefix: string) => pathname === prefix || pathname.startsWith(`${prefix}/`);

/** The five places, in order; one is current when the path is in its section. */
export function navItems(pathname: string): NavItem[] {
	return ORDER.map((key) => ({
		key,
		href: NAV_TARGETS[key],
		label: strings.nav[key],
		current:
			key === 'home'
				? pathname === '/'
				: key === 'new'
					? false
					: SECTIONS[key].some((prefix) => under(pathname, prefix))
	}));
}

const AUTH_PATHS = ['/login', '/setup', '/invite', '/reset', '/link', '/link-player'];

/** How much app chrome a screen gets; the first match wins. */
export function chromeOf(pathname: string): Chrome {
	if (/^\/(s|v|c|cv)\//.test(pathname)) return 'bare';
	if (/^\/(b|j|e|p)\//.test(pathname)) return 'standalone';
	if (AUTH_PATHS.includes(pathname)) return 'auth';
	return 'app';
}

/** The page a back link leads to, or null on a top-level page. */
export function parentOf(pathname: string): Parent | null {
	const { nav } = strings;
	if (pathname.startsWith('/events/')) return { href: NAV_TARGETS.organise, label: nav.organise };
	if (pathname.startsWith('/players/')) return { href: '/players', label: nav.people };
	if (pathname === '/history') return { href: NAV_TARGETS.results, label: nav.results };
	if (pathname === '/admin') return { href: '/account', label: nav.account };
	if (
		pathname === '/account' ||
		pathname === '/new' ||
		pathname.startsWith('/new/') ||
		pathname === '/boards/new' ||
		pathname === '/calendar'
	)
		return { href: '/', label: nav.home };
	return null;
}

/** Up to two initials for the account circle: first letters of the first and last word. */
export function initials(name: string): string {
	const words = name.trim().split(/\s+/).filter(Boolean);
	if (words.length === 0) return '?';
	const first = words[0]!;
	const last = words.length > 1 ? words[words.length - 1]! : '';
	return (Array.from(first)[0]! + (Array.from(last)[0] ?? '')).toUpperCase();
}
