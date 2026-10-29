import { describe, expect, it } from 'vitest';
import { bySegment, organiseRows, segmentFromQuery, signupsOpen, type OrganiseRow } from './organise';
import type { Organised, OrganisedBoard, OrganisedEvent, OrganisedMatch } from './types';

const NOW = 1_790_000_000;
const H = 3600;

const event = (
	over: Partial<OrganisedEvent> = {},
	schedule: Partial<OrganisedEvent['schedule']> = {}
): OrganisedEvent => ({
	id: 'e1',
	name: 'Spring cup',
	kind: 'tournament',
	format: {} as OrganisedEvent['format'],
	settings: {},
	status: 'setup',
	created_at: NOW - 5 * H,
	started_at: null,
	finished_at: null,
	team_count: 0,
	court_count: 2,
	schedule: {
		starts_at: NOW + 24 * H,
		place: null,
		signups: 'off',
		min_teams: 2,
		max_teams: null,
		deadline_at: null,
		...schedule
	},
	cancelled_at: null,
	signups_closed_at: null,
	join_token: null,
	signed_teams: 0,
	solo_count: 0,
	line_count: 0,
	...over
});

const match = (over: Partial<OrganisedMatch> = {}): OrganisedMatch => ({
	id: 'm1',
	join_token: 'jt',
	starts_at: NOW + 2 * H,
	place: null,
	state: 'open',
	places: ['A', 'B', 'C', 'D'],
	line_count: 0,
	...over
});

const board = (over: Partial<OrganisedBoard> = {}): OrganisedBoard => ({
	token: 'bt',
	title: 'Friday board',
	place: null,
	starts_at: NOW + H,
	ends_at: NOW + 3 * H,
	over: false,
	member_count: 6,
	game_count: 0,
	...over
});

const rows = (o: Partial<Organised>, tz = 'UTC') =>
	organiseRows({ events: [], matches: [], boards: [], ...o }, NOW, tz);
const one = (o: Partial<Organised>): OrganiseRow => {
	const [r] = rows(o);
	if (!r) throw new Error('no row');
	return r;
};

describe('organiseRows: events', () => {
	it('puts setup in coming, running in running, finished and cancelled in past', () => {
		expect(one({ events: [event({ status: 'setup' })] }).segment).toBe('coming');
		expect(one({ events: [event({ status: 'running' })] }).segment).toBe('running');
		expect(one({ events: [event({ status: 'finished' })] }).segment).toBe('past');
		expect(one({ events: [event({ status: 'cancelled' })] }).segment).toBe('past');
	});
	it('builds key, href, title, chip and sortAt', () => {
		const r = one({ events: [event()] });
		expect(r).toMatchObject({
			kind: 'event',
			key: 'event:e1',
			href: '/events/e1',
			title: 'Spring cup',
			chip: 'Setting up',
			sortAt: NOW + 24 * H
		});
	});
	it('chips Sign-ups for a setup event with sign-ups on, else the status', () => {
		expect(one({ events: [event({}, { signups: 'public' })] }).chip).toBe('Sign-ups');
		expect(one({ events: [event({ status: 'running' }, { signups: 'public' })] }).chip).toBe('Running');
	});
	it('falls back to created_at and No date yet without a start', () => {
		const r = one({ events: [event({}, { starts_at: null })] });
		expect(r.sortAt).toBe(NOW - 5 * H);
		expect(r.when).toBe('No date yet');
	});
	it('writes when as date and time, and the place', () => {
		const r = one({ events: [event({}, { starts_at: 1_793_131_200, place: 'Club' })] });
		expect(r.when).toBe('Tue 27 Oct · 20:00');
		expect(r.place).toBe('Club');
	});
	it('needs: start overdue first', () => {
		const e = event({ solo_count: 2 }, { starts_at: NOW - 60 });
		expect(one({ events: [e] }).needs).toBe('Start overdue');
	});
	it('needs: solos waiting, singular and plural', () => {
		expect(one({ events: [event({ solo_count: 1 })] }).needs).toBe('1 solo waiting to pair');
		expect(one({ events: [event({ solo_count: 3 })] }).needs).toBe('3 solo waiting to pair');
	});
	it('needs: sign-ups close at a future deadline', () => {
		const deadline = 1_793_131_200;
		const r = one({ events: [event({ line_count: 4 }, { signups: 'public', deadline_at: deadline })] });
		expect(r.needs).toBe('Sign-ups close Tue 27 Oct');
	});
	it('needs: ignores a past deadline and sign-ups off', () => {
		expect(one({ events: [event({ line_count: 4 }, { signups: 'public', deadline_at: NOW - 1 })] }).needs).toBe(
			'4 in line'
		);
		expect(one({ events: [event({ line_count: 4 }, { signups: 'off', deadline_at: NOW + H })] }).needs).toBe(
			'4 in line'
		);
	});
	it('closed early: no "Sign-ups close", status chip, line_count needs', () => {
		const e = event({ line_count: 4, signups_closed_at: NOW - H }, { signups: 'public', deadline_at: NOW + 24 * H });
		const r = one({ events: [e] });
		expect(r.needs).toBe('4 in line');
		expect(r.chip).toBe('Setting up');
	});
	it('needs: people in line', () => {
		expect(one({ events: [event({ line_count: 2 })] }).needs).toBe('2 in line');
	});
	it('needs: null when nothing applies, and for running or past events', () => {
		expect(one({ events: [event()] }).needs).toBeNull();
		expect(one({ events: [event({ status: 'running', solo_count: 2 })] }).needs).toBeNull();
		expect(one({ events: [event({ status: 'finished', line_count: 2 })] }).needs).toBeNull();
	});
});

describe('organiseRows: matches', () => {
	it('is coming for open and full, and for a recent start', () => {
		expect(one({ matches: [match()] }).segment).toBe('coming');
		expect(one({ matches: [match({ state: 'full' })] }).segment).toBe('coming');
		expect(one({ matches: [match({ state: 'open', starts_at: NOW - 2 * H })] }).segment).toBe('coming');
	});
	it('is past for cancelled, started and past', () => {
		for (const state of ['cancelled', 'started', 'past'] as const) {
			expect(one({ matches: [match({ state })] }).segment).toBe('past');
		}
	});
	it('is past for open or full three hours after the start', () => {
		expect(one({ matches: [match({ state: 'open', starts_at: NOW - 3 * H })] }).segment).toBe('past');
		expect(one({ matches: [match({ state: 'full', starts_at: NOW - 3 * H })] }).segment).toBe('past');
	});
	it('builds title, chip, href, key and sortAt', () => {
		const r = one({ matches: [match({ place: 'Court 2', state: 'full' })] });
		expect(r).toMatchObject({
			kind: 'match',
			key: 'match:m1',
			title: 'Match · Court 2',
			place: 'Court 2',
			chip: 'Full',
			href: '/j/jt',
			sortAt: NOW + 2 * H
		});
		expect(one({ matches: [match()] }).title).toBe('Match');
	});
	it('needs: open places', () => {
		expect(one({ matches: [match({ places: ['A', null, null, 'D'] })] }).needs).toBe('2 more needed');
	});
	it('needs: people in line when the match is full', () => {
		expect(one({ matches: [match({ line_count: 3 })] }).needs).toBe('3 in line');
	});
	it('needs: null, and null in past', () => {
		expect(one({ matches: [match()] }).needs).toBeNull();
		expect(one({ matches: [match({ state: 'started', places: [null, null, null, null] })] }).needs).toBeNull();
	});
});

describe('organiseRows: boards', () => {
	it('is coming until over', () => {
		expect(one({ boards: [board()] }).segment).toBe('coming');
		expect(one({ boards: [board({ over: true })] }).segment).toBe('past');
	});
	it('builds title, chip, href, key and sortAt', () => {
		expect(one({ boards: [board({ place: 'Club' })] })).toMatchObject({
			kind: 'board',
			key: 'board:bt',
			title: 'Friday board',
			place: 'Club',
			chip: 'Board',
			href: '/b/bt',
			sortAt: NOW + H
		});
	});
	it('needs: no games yet, one game, several games', () => {
		expect(one({ boards: [board()] }).needs).toBe('No games yet');
		expect(one({ boards: [board({ game_count: 1 })] }).needs).toBe('1 game formed');
		expect(one({ boards: [board({ game_count: 3 })] }).needs).toBe('3 games formed');
	});
	it('needs: null once over', () => {
		expect(one({ boards: [board({ over: true })] }).needs).toBeNull();
	});
});

describe('bySegment', () => {
	const mk = (key: string, segment: OrganiseRow['segment'], sortAt: number): OrganiseRow => ({
		kind: 'board',
		key,
		title: key,
		when: '',
		place: null,
		chip: '',
		needs: null,
		href: '/',
		segment,
		sortAt
	});
	const all = [
		mk('a', 'coming', 30),
		mk('b', 'coming', 10),
		mk('c', 'past', 10),
		mk('d', 'past', 30),
		mk('e', 'running', 20),
		mk('f', 'running', 5)
	];
	it('sorts coming ascending', () => {
		expect(bySegment(all, 'coming').map((r) => r.key)).toEqual(['b', 'a']);
	});
	it('sorts running ascending', () => {
		expect(bySegment(all, 'running').map((r) => r.key)).toEqual(['f', 'e']);
	});
	it('sorts past descending', () => {
		expect(bySegment(all, 'past').map((r) => r.key)).toEqual(['d', 'c']);
	});
});

describe('segmentFromQuery', () => {
	it('reads seg', () => {
		expect(segmentFromQuery(new URLSearchParams('seg=running'))).toBe('running');
		expect(segmentFromQuery(new URLSearchParams('seg=past'))).toBe('past');
		expect(segmentFromQuery(new URLSearchParams('seg=coming'))).toBe('coming');
	});
	it('defaults to coming', () => {
		expect(segmentFromQuery(new URLSearchParams(''))).toBe('coming');
		expect(segmentFromQuery(new URLSearchParams('seg=nope'))).toBe('coming');
	});
});

describe('signupsOpen', () => {
	it('is open only with sign-ups on, not closed and the deadline ahead', () => {
		expect(signupsOpen(event({}, { signups: 'public' }), NOW)).toBe(true);
		expect(signupsOpen(event({}, { signups: 'off' }), NOW)).toBe(false);
		expect(signupsOpen(event({ signups_closed_at: NOW - 1 }, { signups: 'public' }), NOW)).toBe(false);
		expect(signupsOpen(event({}, { signups: 'public', deadline_at: NOW - 1 }), NOW)).toBe(false);
	});
});
