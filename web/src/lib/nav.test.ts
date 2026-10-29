import { describe, expect, it } from 'vitest';
import { chromeOf, initials, navItems, parentOf } from './nav';

const current = (path: string) =>
	navItems(path)
		.filter((i) => i.current)
		.map((i) => i.key);

describe('navItems', () => {
	it('lists the five places in order', () => {
		expect(navItems('/').map((i) => [i.key, i.href])).toEqual([
			['home', '/'],
			['organise', '/organise'],
			['new', '/new'],
			['results', '/results'],
			['people', '/players']
		]);
	});
	it('marks only Home on /', () => expect(current('/')).toEqual(['home']));
	it('marks Organise on the event desk and the event form', () => {
		for (const p of ['/events', '/events/abc', '/organise', '/new/event']) expect(current(p)).toEqual(['organise']);
	});
	it('marks Results on history, profile and results', () => {
		for (const p of ['/history', '/profile', '/results']) expect(current(p)).toEqual(['results']);
	});
	it('marks People on a player', () => expect(current('/players/p1')).toEqual(['people']));
	it('never marks New', () => {
		for (const p of ['/new', '/new/match', '/new/board']) expect(current(p)).toEqual([]);
	});
	it('marks nothing on a near miss or on account', () => {
		for (const p of ['/eventsfoo', '/account', '/calendar']) expect(current(p)).toEqual([]);
	});
});

describe('chromeOf', () => {
	it('is bare on the fence, viewer and court screens', () => {
		for (const p of ['/s/x', '/v/x', '/c/x', '/cv/x']) expect(chromeOf(p)).toBe('bare');
	});
	it('is standalone on board, sign-up, spectator and profile links', () => {
		for (const p of ['/b/x', '/j/x', '/e/x', '/p/x']) expect(chromeOf(p)).toBe('standalone');
	});
	it('is auth on the sign-in pages', () => {
		for (const p of ['/login', '/setup', '/invite', '/reset', '/link', '/link-player'])
			expect(chromeOf(p)).toBe('auth');
	});
	it('is app everywhere else', () => {
		for (const p of ['/', '/c', '/events', '/account', '/b', '/linked']) expect(chromeOf(p)).toBe('app');
	});
});

describe('parentOf', () => {
	it('has no parent on a top-level page', () => {
		for (const p of ['/', '/events', '/profile', '/history', '/players', '/organise', '/results', '/login', '/b/x'])
			expect(parentOf(p)).toBeNull();
	});
	it('names the parent of nested pages', () => {
		expect(parentOf('/events/e1')).toEqual({ href: '/organise', label: 'Organise' });
		expect(parentOf('/new/event')).toEqual({ href: '/organise', label: 'Organise' });
		expect(parentOf('/players/p1')).toEqual({ href: '/players', label: 'People' });
		expect(parentOf('/account')).toEqual({ href: '/', label: 'Home' });
		expect(parentOf('/admin')).toEqual({ href: '/account', label: 'Account' });
		for (const p of ['/new', '/new/match', '/boards/new']) expect(parentOf(p)).toEqual({ href: '/', label: 'Home' });
	});
});

describe('initials', () => {
	it('takes the first and last word', () => {
		expect(initials('Ana Maria Silva')).toBe('AS');
		expect(initials('coach')).toBe('C');
	});

	it('survives an empty name', () => {
		expect(initials('  ')).toBe('?');
	});
});
